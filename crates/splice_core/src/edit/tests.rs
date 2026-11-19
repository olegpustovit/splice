use super::*;
use crate::package::Package;
use std::path::Path;

pub(super) fn sample() -> Project {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
    let path = std::env::temp_dir().join(format!("splice-edit-{}.splice", std::process::id()));
    std::fs::create_dir_all(path.join("transcripts")).unwrap();
    for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
        std::fs::copy(source.join(file), path.join(file)).unwrap();
    }
    let (package, project) = Package::open(&path).unwrap();
    drop(package);
    std::fs::remove_dir_all(path).unwrap();
    project
}

#[test]
fn apply_matches_prototype_refs_mints_and_cascade() {
    let base = sample();
    let cases =
        splice_json::parse(include_str!("../../../../fixtures/json/apply-parity.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let ops = case.field("ops").unwrap().as_array().unwrap();
        let expected = case.field("result").unwrap();
        let mut project = base.clone();
        let report = apply(&mut project, ops, Source::Human, false).unwrap();
        let result = parity(&project, &report);
        let mut expected_bytes = String::new();
        let mut actual_bytes = String::new();
        splice_json::write(expected, &mut expected_bytes).unwrap();
        splice_json::write(&result, &mut actual_bytes).unwrap();
        assert_eq!(actual_bytes, expected_bytes);
        let mut replayed = base.clone();
        let replay_report = replay(&mut replayed, ops, Source::Human, &report.mints).unwrap();
        assert_eq!(report, replay_report);
        assert_eq!(parity(&replayed, &replay_report), result);
    }
}

#[test]
fn apply_is_atomic_on_failure_and_dry_run() {
    let mut project = sample();
    let mut workspace = ApplyWorkspace::default();
    let batch = splice_json::parse(
        r#"[
        {"cmd":"item.split","item":"clp_take","at":372,"ref":"$cut"},
        {"cmd":"overlay.add","kind":"title","at":"$nope"}
    ]"#,
    )
    .unwrap();
    let before = format!("{project:?}");
    let error = workspace
        .apply(
            &mut project,
            batch.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap_err();
    assert_eq!(error.op, Some(1));
    assert_eq!(error.error.code, "BAD_ARGS");
    assert_eq!(format!("{project:?}"), before);

    let ops = &batch.as_array().unwrap()[..1];
    let dry = workspace
        .apply(&mut project, ops, Source::Human, true)
        .unwrap()
        .clone();
    assert_eq!(format!("{project:?}"), before);
    assert!(!dry.diff.is_empty());
    assert_ne!(dry.mints.len(), 0);
    let live = workspace
        .apply(&mut project, ops, Source::Human, false)
        .unwrap();
    assert_eq!(live, &dry);

    let mut cold = sample();
    assert_eq!(apply(&mut cold, ops, Source::Human, false).unwrap(), dry);
    assert_eq!(format!("{project:?}"), format!("{cold:?}"));

    let mut original = sample();
    let error = replay(&mut original, ops, Source::Human, &[]).unwrap_err();
    assert_eq!(error.error.code, "REPLAY_MINTS");
    assert_eq!(format!("{original:?}"), before);
}

fn parity(project: &Project, report: &ApplyReport) -> Json {
    let clips = project
        .clips
        .order()
        .iter()
        .map(|&slot| {
            let timing = project.clips.timing(slot).unwrap();
            Json::Array(vec![
                str_json(&project.clips.id(slot).unwrap()),
                frame(timing.start),
                frame(timing.length),
                frame(timing.source),
            ])
        })
        .collect();
    let items = project
        .items
        .order()
        .iter()
        .map(|&slot| {
            let id = project.items.id(slot).unwrap();
            let place = span(project, TimelineId::Item(id)).unwrap();
            Json::Array(vec![
                str_json(&id),
                frame(place.start),
                frame(place.end - place.start),
                str_json(&place.lane.unwrap()),
            ])
        })
        .collect();
    let created = report
        .changes
        .iter()
        .map(|change| {
            Json::Array(
                change
                    .created
                    .iter()
                    .map(|id| match id {
                        Target::Timeline(id) => str_json(id),
                        Target::Lane(id) => str_json(id),
                        _ => panic!("unexpected creation: {id:?}"),
                    })
                    .collect(),
            )
        })
        .collect();
    let effects = report
        .side_effects
        .iter()
        .map(|effect| match effect {
            SideEffect::Removed { id, cascade_from } => object(vec![
                ("id", str_json(id)),
                ("deleted", Json::Bool(true)),
                ("cascadeFrom", str_json(&cascade_from.expect("cascade"))),
            ]),
            SideEffect::Bumped { id, from, to } => object(vec![
                ("id", str_json(id)),
                (
                    "bumped",
                    object(vec![("from", str_json(from)), ("to", str_json(to))]),
                ),
            ]),
            SideEffect::Reanchored { id, from, to } => {
                let (Anchor::ClipSource { clip: from, .. }, Anchor::ClipSource { clip: to, .. }) =
                    (from, to)
                else {
                    panic!("expected a clip-source reanchor");
                };
                object(vec![
                    ("id", str_json(id)),
                    (
                        "reanchored",
                        object(vec![("from", str_json(from)), ("to", str_json(to))]),
                    ),
                ])
            }
        })
        .collect();
    object(vec![
        (
            "mints",
            Json::Array(report.mints.iter().cloned().map(Json::String).collect()),
        ),
        ("clips", Json::Array(clips)),
        ("items", Json::Array(items)),
        ("created", Json::Array(created)),
        ("effects", Json::Array(effects)),
    ])
}

fn object(values: Vec<(&str, Json)>) -> Json {
    Json::Object(
        values
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

fn frame(frame: Frame) -> Json {
    Json::Signed(i64::from(frame))
}

fn str_json(value: &dyn fmt::Display) -> Json {
    Json::String(value.to_string())
}

#[test]
fn apply_resolves_word_and_marker_anchors_and_replays_recorded_ids() {
    let ops = splice_json::parse(r#"[
        {"cmd":"marker.add","at":{"word":"w12","transcriptHash":"3650c8864f4c43ec","edge":"end","offset":1},"ref":"$beat"},
        {"cmd":"lane.create","kind":"graphics","ref":"$lane"},
        {"cmd":"overlay.add","kind":"title","anchor":{"marker":"$beat","offset":2},"length":5,"lane":"$lane"}
    ]"#).unwrap();
    let ops = ops.as_array().unwrap();
    let mut live = sample();
    let mut replayed = live.clone();
    let report = apply(&mut live, ops, Source::Agent, false).unwrap();
    assert_eq!(report.mints, ["mrk_5k", "lan_5l", "ovl_5m"]);
    let marker = MarkerId::from_serial(200);
    let item = ItemId::from_serial(202);
    assert_eq!(
        live.markers.iter().find(|m| m.id == marker).unwrap().at,
        301
    );
    assert_eq!(
        span(&live, TimelineId::Item(item)).unwrap(),
        Span {
            start: 303,
            end: 308,
            lane: Some(LaneId::from_serial(201)),
        }
    );
    assert_eq!(
        live.items
            .timing(live.items.find(item).unwrap())
            .unwrap()
            .anchor,
        Anchor::Marker { marker, offset: 2 }
    );

    // Replaying a retained batch after earlier history has advanced its serial
    // still uses the recorded IDs.
    replayed.serial = 900;
    assert_eq!(
        replay(&mut replayed, ops, Source::Agent, &report.mints).unwrap(),
        report
    );
    assert_eq!(replayed.serial, 900);
    assert_eq!(span(&replayed, TimelineId::Item(item)).unwrap().start, 303);
}

#[test]
fn apply_refuses_explicit_lane_conflict_and_stale_transcript_atomically() {
    let mut project = sample();
    for (raw, code, op) in [
        (
            r#"[{"cmd":"overlay.add","kind":"title","at":2,"length":20,"lane":"lan_gfx"}]"#,
            "LANE_CONFLICT",
            None,
        ),
        (
            r#"[{"cmd":"marker.add","at":{"word":"w12","transcriptHash":"0000000000000000"}}]"#,
            "TRANSCRIPT_CHANGED",
            Some(0),
        ),
    ] {
        let before = format!("{project:?}");
        let batch = splice_json::parse(raw).unwrap();
        let error = apply(
            &mut project,
            batch.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap_err();
        assert_eq!((error.error.code, error.op), (code, op));
        assert_eq!(format!("{project:?}"), before);
    }
}

#[test]
fn tier_a_described_examples_match_prototype_snapshots() {
    let cases = splice_json::parse(include_str!("../../../../fixtures/json/tier-a.json")).unwrap();
    let cases = cases.as_array().unwrap();
    let base = sample();
    let mut expected = String::new();
    let mut actual = String::new();
    for command in commands::COMMANDS
        .iter()
        .filter(|row| row.tier == commands::Tier::First)
    {
        let case = cases
            .iter()
            .find(|case| {
                case.field("op").unwrap().field("cmd").unwrap().as_str() == Some(command.id)
            })
            .unwrap_or_else(|| panic!("missing golden for {}", command.id));
        let described = commands::describe(Some(command.id), None).unwrap();
        let op = described.field("example").unwrap();
        assert_eq!(Some(op), case.field("op"), "{}", command.id);
        assert!(commands::decode(op).is_ok(), "{}", command.id);
        let mut project = base.clone();
        let report = apply(&mut project, std::slice::from_ref(op), Source::Agent, false)
            .unwrap_or_else(|error| panic!("{}: {error}", command.id));
        assert_eq!(report.changes[0].cmd, command.id);
        let snapshot = tier_a_snapshot(&project, report.mints);
        splice_json::write(case.field("snapshot").unwrap(), &mut expected).unwrap();
        splice_json::write(&snapshot, &mut actual).unwrap();
        assert_eq!(actual, expected, "{}", command.id);
    }
    assert_eq!(cases.len(), 20);
}

fn tier_a_snapshot(project: &Project, mints: Vec<String>) -> Json {
    let encoded = crate::package::project::encode(project);
    let array = |key| encoded.field(key).unwrap().as_array().unwrap();
    let lanes = array("lanes")
        .iter()
        .map(|lane| Json::Array(columns(lane, &["id", "kind", "name"])))
        .collect();
    let clips = array("clips")
        .iter()
        .map(|clip| {
            let mut cells = columns(
                clip,
                &[
                    "id", "media", "start", "length", "source", "speed", "fadeIn", "fadeOut",
                    "name", "color",
                ],
            );
            let mut enabled: Vec<_> = clip
                .field("enabled")
                .unwrap()
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, _)| name)
                .collect();
            enabled.sort_unstable();
            cells.push(Json::Array(
                enabled.into_iter().cloned().map(Json::String).collect(),
            ));
            Json::Array(cells)
        })
        .collect();
    let items = array("items")
        .iter()
        .map(|item| {
            let mut cells = columns(item, &["id", "kind", "lane"]);
            let id = item.field("id").unwrap().as_str().unwrap().parse().unwrap();
            cells.push(frame(span(project, TimelineId::Item(id)).unwrap().start));
            cells.push(item.field("length").unwrap().clone());
            for fade in ["fadeIn", "fadeOut"] {
                cells.push(item.field(fade).cloned().unwrap_or(frame(0)));
            }
            cells.extend(columns(item, &["name", "color"]));
            let anchor = item.field("anchor").unwrap();
            let anchor = if anchor.field("pinned").is_some() {
                vec![str_json(&"pinned"), anchor.field("frame").unwrap().clone()]
            } else {
                vec![
                    str_json(&"source"),
                    anchor.field("clip").unwrap().clone(),
                    anchor.field("sourceFrame").unwrap().clone(),
                ]
            };
            cells.push(Json::Array(anchor));
            let keys: &[&str] = match item.field("kind").unwrap().as_str().unwrap() {
                "title" => &["text"],
                "media" => &["media", "source", "placement", "behindPresenter", "muted"],
                "motion" => &["from", "to"],
                "caption" => &["words"],
                kind => panic!("unexpected item kind {kind}"),
            };
            cells.push(Json::Array(columns(item.field("params").unwrap(), keys)));
            Json::Array(cells)
        })
        .collect();
    let markers = array("markers")
        .iter()
        .map(|marker| Json::Array(columns(marker, &["id", "at", "name"])))
        .collect();
    object(vec![
        ("aspect", encoded.field("aspect").unwrap().clone()),
        ("access", encoded.field("access").unwrap().clone()),
        (
            "mints",
            Json::Array(mints.into_iter().map(Json::String).collect()),
        ),
        ("lanes", Json::Array(lanes)),
        ("clips", Json::Array(clips)),
        ("items", Json::Array(items)),
        ("markers", Json::Array(markers)),
    ])
}

fn columns(value: &Json, keys: &[&str]) -> Vec<Json> {
    keys.iter()
        .map(|key| value.field(key).cloned().unwrap_or(Json::Null))
        .collect()
}

#[test]
fn tier_a_failed_edits_leave_the_project_unchanged() {
    let mut project = sample();
    for (raw, code) in [
        (
            r#"{"cmd":"item.move","items":"clp_end","to":300,"ripple":false}"#,
            "OVERLAP",
        ),
        (
            r#"{"cmd":"item.trim","item":"clp_take","edge":"out","delta":121}"#,
            "OUT_OF_SOURCE",
        ),
        (
            r#"{"cmd":"lane.delete","lane":"lan_gfx"}"#,
            "LANE_NOT_EMPTY",
        ),
        (
            r#"{"cmd":"project.set","aspect":"4:5","access":"off"}"#,
            "PERMISSION_DENIED",
        ),
        (
            r#"{"cmd":"item.anchor","item":"ovl_broll","anchor":{"item":"ovl_broll"}}"#,
            "BAD_ARGS",
        ),
    ] {
        let op = splice_json::parse(raw).unwrap();
        let before = format!("{project:?}");
        let error = apply(&mut project, &[op], Source::Agent, false).unwrap_err();
        assert_eq!(error.error.code, code, "{raw}: {error}");
        assert_eq!(format!("{project:?}"), before);
    }
}

#[test]
fn tier_a_duplicate_preserves_original_effect_ids() {
    let mut project = sample();
    let op = splice_json::parse(r#"{"cmd":"item.duplicate","items":"clp_open","at":0}"#).unwrap();
    let original: ClipId = "clp_open".parse().unwrap();
    let data = |project: &Project, id| {
        project
            .clips
            .data(project.clips.find(id).unwrap())
            .unwrap()
            .appearance
            .effects[0]
            .id
    };
    let original_effect = data(&project, original);
    let report = apply(&mut project, &[op], Source::Agent, false).unwrap();
    let Target::Timeline(TimelineId::Clip(copied)) = report.changes[0].created[0] else {
        panic!("expected a copied clip");
    };
    assert_eq!(span(&project, TimelineId::Clip(copied)).unwrap().start, 0);
    assert_eq!(
        span(&project, TimelineId::Clip(original)).unwrap().start,
        120
    );
    assert_eq!(data(&project, original), original_effect);
    assert_ne!(data(&project, copied), original_effect);
}

#[test]
fn tier_a_magnetic_move_and_ripple_trim_keep_spine_contiguous() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[
        {"cmd":"item.move","items":"clp_end","to":100},
        {"cmd":"item.trim","item":"clp_take","edge":"in","delta":20},
        {"cmd":"clip.speed","items":"clp_end","speed":16}
    ]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let clips: Vec<_> = project
        .clips
        .order()
        .iter()
        .map(|&slot| {
            let timing = project.clips.timing(slot).unwrap();
            (
                project.clips.id(slot).unwrap().to_string(),
                timing.start,
                timing.length,
                timing.source,
            )
        })
        .collect();
    assert_eq!(
        clips,
        [
            ("clp_open".into(), 0, 120, 0),
            ("clp_end".into(), 120, 7, 0),
            ("clp_take".into(), 127, 400, 20)
        ]
    );
    let label = "ovl_label".parse().unwrap();
    assert_eq!(span(&project, TimelineId::Item(label)).unwrap().start, 127);
}

#[test]
fn tier_a_project_aspect_and_marker_swatch_persist_in_snapshot() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[
      {"cmd":"project.set","aspect":"4:5","access":"read"},
      {"cmd":"item.set","items":"mrk_2","color":"green"}
    ]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Human, false).unwrap();
    assert_eq!(project.size, [1080, 1350]);
    assert_eq!(project.access, Access::Read);
    let snapshot = crate::package::project::encode(&project);
    let markers = snapshot.field("markers").unwrap().as_array().unwrap();
    assert_eq!(markers[1].field("color").unwrap().as_str(), Some("green"));
    let next =
        splice_json::parse(r#"{"cmd":"marker.set","marker":"mrk_2","name":"Save beat"}"#).unwrap();
    assert_eq!(
        apply(&mut project, &[next], Source::Agent, false)
            .unwrap_err()
            .error
            .code,
        "PERMISSION_DENIED"
    );
}

#[test]
fn tier_a_fractional_trim_keeps_the_clip_inside_its_media() {
    let mut project = sample();
    let id = "clp_end".parse().unwrap();
    let slot = project.clips.find(id).unwrap();
    let timing = project.clips.timing_mut(slot).unwrap();
    timing.speed = 1.5;
    timing.length = 80;
    let op = splice_json::parse(r#"{"cmd":"item.trim","item":"clp_end","edge":"in","delta":1}"#)
        .unwrap();
    apply(&mut project, &[op], Source::Human, false).unwrap();
    let timing = project.clips.timing(slot).unwrap();
    assert_eq!((timing.length, timing.source), (79, 1));
}

fn budget_project() -> Project {
    let mut project = sample();
    let lane = LaneId::from_serial(11);
    project.lanes.push(Lane {
        id: lane,
        name: "Budget titles".into(),
        kind: LaneKind::Graphics,
        effects: Vec::new(),
    });
    let clip = "clp_take".parse().unwrap();
    for index in 0..2000 - project.items.order().len() {
        let start = 800 + i32::try_from(index).unwrap() * 20;
        let anchor = if index < 30 {
            Anchor::ClipSource {
                clip,
                source: i32::try_from(index).unwrap() * 12,
            }
        } else {
            Anchor::Pinned(start)
        };
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(10_000 + index as u64),
                timing: ItemTiming { anchor, length: 8 },
                data: ItemData {
                    lane,
                    kind: ItemKind::Title {
                        text: format!("Item {index}"),
                    },
                    appearance: Appearance::default(),
                },
            })
            .unwrap();
    }
    let slot = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let appearance = &mut project.clips.data_mut(slot).unwrap().appearance;
    appearance.name = Some("Edited take".into());
    appearance.color = Some("orange".into());
    project
}

fn budget_ops() -> [Json; 6] {
    [
        r#"{"cmd":"item.trim","item":"clp_take","edge":"out","delta":-1}"#,
        r#"{"cmd":"item.move","items":"ovl_broll","delta":1}"#,
        r#"{"cmd":"item.set","items":"clp_take","name":"Edited take","color":"orange","fadeOut":7}"#,
        r#"{"cmd":"item.trim","item":"clp_take","edge":"out","delta":1}"#,
        r#"{"cmd":"item.move","items":"ovl_broll","delta":-1}"#,
        r#"{"cmd":"item.set","items":"clp_take","name":"Clean take","color":"blue","fadeOut":0}"#,
    ]
    .map(|op| splice_json::parse(op).unwrap())
}

#[test]
fn budget_warm_trim_move_set_allocate_nothing() {
    let mut project = budget_project();
    let mut workspace = ApplyWorkspace::default();
    let operations = budget_ops();
    let meter = splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
    meter.warm(&mut || {
        for op in &operations {
            let report = workspace
                .apply(&mut project, std::slice::from_ref(op), Source::Human, false)
                .unwrap();
            assert!(!report.diff.is_empty());
        }
    });
}

#[test]
fn budget_warm_magnetic_move_and_marker_order_allocate_nothing() {
    let mut project = budget_project();
    for (serial, at) in [(3, 400), (4, 500)] {
        project.markers.push(Marker {
            id: MarkerId::from_serial(serial),
            at,
            name: String::new(),
            color: None,
        });
    }
    let mut workspace = ApplyWorkspace::default();
    let operations = [
        r#"{"cmd":"item.move","items":"clp_end","to":100}"#,
        r#"{"cmd":"item.move","items":"mrk_1","to":500}"#,
    ]
    .map(|op| splice_json::parse(op).unwrap());
    let meter = splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
    meter.warm(&mut || {
        for op in &operations {
            let report = workspace
                .apply(&mut project, std::slice::from_ref(op), Source::Human, true)
                .unwrap();
            assert!(!report.diff.is_empty());
        }
    });
    workspace
        .apply(&mut project, &operations[1..], Source::Human, false)
        .unwrap();
    let markers: Vec<_> = project
        .markers
        .iter()
        .map(|marker| marker.id.serial())
        .collect();
    assert_eq!(markers, [2, 3, 1, 4]);
}

#[test]
fn budget_apply_p99_under_one_millisecond() {
    use std::time::{Duration, Instant};
    let mut project = budget_project();
    let operations = budget_ops();
    let mut workspace = ApplyWorkspace::default();
    let mut times = Vec::with_capacity(300);
    assert_eq!(project.items.order().len(), 2_000);
    for i in 0..306 {
        let at = Instant::now();
        std::hint::black_box(
            workspace
                .apply(
                    &mut project,
                    std::slice::from_ref(&operations[i % 6]),
                    Source::Human,
                    false,
                )
                .unwrap(),
        );
        if i >= 6 {
            times.push(at.elapsed());
        }
    }
    times.sort_unstable();
    let p99 = times[times.len() * 99 / 100 - 1];
    eprintln!(
        "apply on {} items: p99 {p99:?} ({} samples)",
        project.items.order().len(),
        times.len()
    );
    assert!(p99 < Duration::from_millis(1));
}
