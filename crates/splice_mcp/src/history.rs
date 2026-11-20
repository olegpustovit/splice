use super::{Json, object, project::access, text, wire::*};
use splice_core::{
    commands::Target,
    edit::{self, Cause, Recropped, SideEffect, Span},
    history::{EditResult, Scope, Store, StoreError, WriteOptions},
    package::{
        self,
        journal::{Change, Source},
    },
};

pub(super) fn start(store: &mut Store, args: &Json) -> Result<Json, StoreError> {
    let agent = args.optional_string("agent")?.unwrap_or("mcp");
    if agent.trim().is_empty() {
        return Err(bad("agent", "a nonempty client name"));
    }
    let session = store.start_session(agent, now())?;
    Ok(object(vec![
        ("sessionId", display(&session.id)),
        ("checkpoint", rev(session.checkpoint)),
        ("revision", rev(store.project().rev)),
        ("access", access(store.project().access)),
    ]))
}

pub(super) fn apply(store: &mut Store, args: &Json) -> Result<Json, StoreError> {
    let options = write_options(args)?;
    let profile = super::lints::profile_arg(args)?;
    let ops = args
        .field("ops")
        .and_then(Json::as_array)
        .filter(|ops| !ops.is_empty())
        .ok_or_else(|| {
            bad(
                "ops",
                "a nonempty array of operations from commands.describe",
            )
        })?;
    let mut result = apply_batch(store, ops, &options)?;
    // Lints read the project the batch leads to (D18): the staged one for a dry run.
    let (lints, pacing) = super::lints::check_profiled(
        result.staged.take().as_deref().unwrap_or(store.project()),
        store.path(),
        profile,
    );
    let mut reply = edit_result_json(result);
    if let Json::Object(fields) = &mut reply {
        fields.push(("lints".into(), Json::Array(lints)));
        fields.push(("pacing".into(), pacing));
    }
    Ok(reply)
}

/// Applies a client edit; MCP, `splice apply` and the viewer all come through here. It fills
/// the inputs the core cannot read (M10), copies consolidated media only for a batch that will
/// commit (M13), and probes local media paths: Lottie and SVG everywhere (T45.2), audio, video
/// and images on macOS.
pub fn apply_batch(
    store: &mut Store,
    ops: &[Json],
    options: &WriteOptions,
) -> Result<EditResult, StoreError> {
    let (package, fps) = (store.path().to_path_buf(), store.project().fps);
    let mut load = |path: &str| {
        if let Some(codec) = super::vector::codec(path) {
            return super::vector::probe(&package, path, codec, fps);
        }
        #[cfg(target_os = "macos")]
        return splice_media::probe(&package, std::path::Path::new(path), fps).map_err(|failure| {
            splice_core::commands::CommandError::new(
                failure.code,
                failure.message,
                failure.fix.into(),
            )
        });
        #[cfg(not(target_os = "macos"))]
        edit::missing_media(path)
    };
    let filled = super::enrich::ops(store, ops, &mut load)?;
    let ops = filled.as_deref().unwrap_or(ops);
    let has = |name: &str| {
        ops.iter()
            .any(|op| op.field("cmd").and_then(Json::as_str) == Some(name))
    };
    let (consolidates, saves) = (has("media.consolidate"), has("look.save"));
    if !options.dry_run && (consolidates || saves) {
        // Files are copied (and looks written) only for a batch that commits: a failing,
        // stale or refused batch writes nothing. A keyed retry of a committed batch replays;
        // its files exist.
        let check = WriteOptions {
            dry_run: true,
            ..options.clone()
        };
        if store.apply_with_media(ops, &check, &mut load).is_ok() {
            if consolidates {
                super::enrich::consolidate(store, ops, &mut load)?;
            }
            if saves {
                super::enrich::save_looks(store, ops, &mut load)?;
            }
        }
    }
    store.apply_with_media(ops, options, &mut load)
}

/// Checkpoints a CLI edit and includes any checkpoint failure in the durable receipt.
pub fn checkpoint_edit(store: &mut Store, result: EditResult) -> Json {
    // An append already synced by Store::apply remains committed if the checkpoint fails.
    let checkpoint = if result.dry_run || result.replayed {
        None
    } else {
        store.checkpoint().err()
    };
    let mut reply = edit_result_json(result);
    if let Some(warning) = checkpoint
        && reply.field("warning").is_none()
        && let Json::Object(fields) = &mut reply
    {
        fields.push(("warning".into(), error_payload(warning)));
    }
    reply
}

/// Serializes a committed edit, replay receipt, or preview for a command client.
pub fn edit_result_json(result: EditResult) -> Json {
    let report = result.report;
    let mut fields = vec![
        ("revision", rev(result.revision)),
        ("hash", hash(result.hash)),
        ("dryRun", Json::Bool(result.dry_run)),
        ("replayed", Json::Bool(result.replayed)),
        (
            "changes",
            Json::Array(
                report
                    .changes
                    .into_iter()
                    .map(|change| {
                        let mut fields = vec![
                            ("op", count(change.op)),
                            ("cmd", text(change.cmd)),
                            ("summary", Json::String(change.summary)),
                            (
                                "created",
                                Json::Array(change.created.iter().map(target).collect()),
                            ),
                        ];
                        let detail = change.detail.unwrap_or_default();
                        if let Some(settle) = detail.settle {
                            fields.push(("settle", Json::Unsigned(settle.ceil() as u64)));
                        }
                        if let Some([end, swept]) = detail.bounds {
                            let rect = |[x, y, w, h]: [f64; 4]| {
                                let round = |v: f64| Json::Float((v * 1e4).round() / 1e4);
                                object(vec![
                                    ("x", round(x)),
                                    ("y", round(y)),
                                    ("w", round(w)),
                                    ("h", round(h)),
                                ])
                            };
                            let inside = |[x, y, w, h]: [f64; 4]| {
                                x >= -1e-9
                                    && y >= -1e-9
                                    && x + w <= 1.0 + 1e-9
                                    && y + h <= 1.0 + 1e-9
                            };
                            fields.push((
                                "bounds",
                                object(vec![
                                    ("final", rect(end)),
                                    ("swept", rect(swept)),
                                    ("onCanvas", Json::Bool(inside(swept))),
                                ]),
                            ));
                        }
                        if let Some(z) = detail.z {
                            fields.push(("z", Json::Signed(i64::from(z))));
                        }
                        object(fields)
                    })
                    .collect(),
            ),
        ),
        (
            "sideEffects",
            Json::Array(report.side_effects.into_iter().map(effect).collect()),
        ),
        (
            "diff",
            Json::Array(
                report
                    .diff
                    .into_iter()
                    .map(|change| {
                        object(vec![
                            ("id", display(&change.id)),
                            ("before", change.before.map_or(Json::Null, span)),
                            ("after", change.after.map_or(Json::Null, span)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "mints",
            Json::Array(report.mints.into_iter().map(Json::String).collect()),
        ),
    ];
    if let Some(error) = result.warning {
        fields.push(("warning", error_payload(error.into())));
    }
    object(fields)
}

pub(super) fn list(store: &Store, args: &Json) -> Result<Json, StoreError> {
    let selected = if args.field("sessionId").is_some() {
        let id = args.session()?;
        Some(
            store
                .sessions()
                .iter()
                .find(|candidate| candidate.id == id)
                .ok_or_else(|| {
                    failure(
                        "NOT_FOUND",
                        &format!("Session {id} does not exist."),
                        "Pass a sessionId returned by session.start.",
                    )
                })?,
        )
    } else {
        None
    };
    let records: Vec<_> = store
        .records()
        .iter()
        .filter(|record| selected.is_none_or(|session| record.batch.session == Some(session.id)))
        .collect();
    let total = records.len();
    let start = args.cursor('h', total)?;
    let end = start.saturating_add(args.limit(100)?).min(total);
    let mut batches = Vec::with_capacity(end - start);
    for record in &records[start..end] {
        let (state, created) = match &record.batch.change {
            Change::Edit { mints, .. } => {
                let state = if store.is_active(record.rev) {
                    "live"
                } else {
                    "reverted"
                };
                (
                    state,
                    Json::Array(mints.iter().map(|id| text(id)).collect()),
                )
            }
            Change::Revert { .. } => ("undo", Json::Array(Vec::new())),
            Change::Restore(_) => ("restored", Json::Array(Vec::new())),
            Change::Undo(_) => ("undo", Json::Array(Vec::new())),
            Change::Redo(_) => ("redo", Json::Array(Vec::new())),
        };
        let (before, after) = (store.at(record.rev - 1)?, store.at(record.rev)?);
        let changes = edit::diff(&before, &after)?;
        batches.push(object(vec![
            ("revision", rev(record.rev)),
            ("summary", text(&record.batch.label)),
            ("source", source(record.batch.source)),
            ("state", text(state)),
            (
                "sessionId",
                record.batch.session.map_or(Json::Null, |id| display(&id)),
            ),
            ("time", Json::Unsigned(record.batch.time)),
            ("created", created),
            (
                "affected",
                Json::Array(changes.iter().map(|change| display(&change.id)).collect()),
            ),
        ]));
    }
    let mut fields = vec![
        ("revision", rev(store.project().rev)),
        ("batches", Json::Array(batches)),
        ("total", count(total)),
        ("nextCursor", next_cursor('h', end, total)),
    ];
    if let Some(session) = selected {
        fields.extend([
            ("sessionId", display(&session.id)),
            ("open", Json::Bool(session.open)),
            ("checkpoint", rev(session.checkpoint)),
            (
                "humanEditsSince",
                count(
                    store
                        .records()
                        .iter()
                        .filter(|r| {
                            r.rev > session.checkpoint
                                && r.batch.source == Source::Human
                                && matches!(r.batch.change, Change::Edit { .. })
                        })
                        .count(),
                ),
            ),
        ]);
    }
    Ok(object(fields))
}

pub(super) fn undo(store: &mut Store, args: &Json) -> Result<Json, StoreError> {
    let scope = match args.string("scope")? {
        "last" => Scope::Last,
        "session" => Scope::Session,
        "to" => Scope::To(args.revision("revision")?),
        _ => return Err(bad("scope", "last, session or to")),
    };
    let options = write_options(args)?;
    if options.key.is_some() {
        return Err(bad(
            "idempotencyKey",
            "omitted for history.undo. Use ifRevision to prevent repeat undo",
        ));
    }
    let result = store.undo(scope, &options)?;
    let mut fields = vec![
        ("revision", rev(result.revision)),
        ("hash", hash(result.hash)),
        ("dryRun", Json::Bool(options.dry_run)),
        (
            "reverted",
            Json::Array(result.reverted.into_iter().map(rev).collect()),
        ),
        ("restored", result.restored.map_or(Json::Null, rev)),
        ("replayed", count(result.replayed)),
        (
            "skipped",
            Json::Array(
                result
                    .skipped
                    .into_iter()
                    .map(|item| {
                        object(vec![
                            ("revision", rev(item.revision)),
                            ("summary", Json::String(item.title)),
                            ("code", Json::String(item.error)),
                            ("message", Json::String(item.message)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    if let Some(error) = result.warning {
        fields.push(("warning", error_payload(error.into())));
    }
    Ok(object(fields))
}

pub(super) fn target(target: &Target) -> Json {
    match target {
        Target::Timeline(id) => display(id),
        Target::Lane(id) => display(id),
        Target::Media(id) => display(id),
        Target::Effect(id) => display(id),
        Target::Project => text("project"),
        Target::Ref(reference) => text(&reference.name),
    }
}

fn span(span: Span) -> Json {
    extend(
        range(span.start, span.end),
        vec![("lane", span.lane.map_or(Json::Null, |id| display(&id)))],
    )
}

fn effect(effect: SideEffect) -> Json {
    let (mut fields, cause) = match effect {
        SideEffect::Removed {
            id,
            cascade_from,
            cause,
        } => (
            vec![
                ("type", text("removed")),
                ("id", display(&id)),
                (
                    "cascadeFrom",
                    cascade_from.map_or(Json::Null, |id| display(&id)),
                ),
            ],
            cause,
        ),
        SideEffect::Reanchored {
            id,
            from,
            to,
            cause,
        } => (
            vec![
                ("type", text("reanchored")),
                ("id", display(&id)),
                ("from", package::anchor(from)),
                ("to", package::anchor(to)),
            ],
            cause,
        ),
        SideEffect::Bumped {
            id,
            from,
            to,
            cause,
        } => (
            vec![
                ("type", text("bumped")),
                ("id", display(&id)),
                ("from", display(&from)),
                ("to", display(&to)),
            ],
            cause,
        ),
        SideEffect::Recropped {
            id,
            owner,
            outside,
            cause,
        } => (
            vec![
                ("type", text("recropped")),
                (
                    "id",
                    match id {
                        Recropped::Zoom(id) => display(&id),
                        Recropped::Mask(id) => display(&id),
                    },
                ),
                ("owner", display(&owner)),
                ("outside", Json::Bool(outside)),
            ],
            cause,
        ),
    };
    fields.push(("cause", text(cause.name())));
    if let Cause::Overlap(holder) = cause {
        fields.push(("with", display(&holder)));
    }
    object(fields)
}
