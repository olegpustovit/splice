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
            SideEffect::Removed {
                id, cascade_from, ..
            } => object(vec![
                ("id", str_json(id)),
                ("deleted", Json::Bool(true)),
                ("cascadeFrom", str_json(&cascade_from.expect("cascade"))),
            ]),
            SideEffect::Bumped { id, from, to, .. } => object(vec![
                ("id", str_json(id)),
                (
                    "bumped",
                    object(vec![("from", str_json(from)), ("to", str_json(to))]),
                ),
            ]),
            SideEffect::Reanchored { id, from, to, .. } => {
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
    // SPLICE_BLESS=1 rewrites the goldens. They began as the JS prototype's snapshots; 0.4
    // diverges on purpose (camera segments replace motion items, titles cut in), so they are
    // Splice's own now and every re-bless is reviewed as a diff.
    let bless = std::env::var_os("SPLICE_BLESS").is_some();
    let mut blessed = Vec::new();
    for command in commands::COMMANDS
        .iter()
        .filter(|row| row.tier == commands::Tier::First)
    {
        let described = commands::describe(Some(command.id), None).unwrap();
        let op = described.field("example").unwrap();
        let case = cases.iter().find(|case| {
            case.field("op").unwrap().field("cmd").unwrap().as_str() == Some(command.id)
        });
        let case = match case {
            Some(case) => case,
            None if bless => &Json::Null,
            None => panic!("missing golden for {}", command.id),
        };
        if !bless {
            assert_eq!(Some(op), case.field("op"), "{}", command.id);
        }
        assert!(commands::decode(op).is_ok(), "{}", command.id);
        let mut project = base.clone();
        let report = apply(&mut project, std::slice::from_ref(op), Source::Agent, false)
            .unwrap_or_else(|error| panic!("{}: {error}", command.id));
        assert_eq!(report.changes[0].cmd, command.id);
        let snapshot = tier_a_snapshot(&project, report.mints);
        if bless {
            blessed.push(Json::Object(vec![
                ("op".into(), op.clone()),
                ("snapshot".into(), snapshot),
            ]));
            continue;
        }
        expected.clear();
        actual.clear();
        splice_json::write(case.field("snapshot").unwrap(), &mut expected).unwrap();
        splice_json::write(&snapshot, &mut actual).unwrap();
        assert_eq!(actual, expected, "{}", command.id);
    }
    if bless {
        let mut text = String::new();
        splice_json::write(&Json::Array(blessed), &mut text).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/json/tier-a.json");
        std::fs::write(path, text).unwrap();
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
                "zoom" => &["amount", "focus", "mode", "spring"],
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
            // 135 source frames after its 15-frame head handle, at 16×.
            ("clp_end".into(), 120, 8, 15),
            ("clp_take".into(), 128, 400, 20)
        ]
    );
    let label = "ovl_label".parse().unwrap();
    assert_eq!(span(&project, TimelineId::Item(label)).unwrap().start, 128);
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
    timing.length = 90;
    let op = splice_json::parse(r#"{"cmd":"item.trim","item":"clp_end","edge":"in","delta":1}"#)
        .unwrap();
    apply(&mut project, &[op], Source::Human, false).unwrap();
    let timing = project.clips.timing(slot).unwrap();
    // 15 + 90 × 1.5 = 150 media frames; the fractional trim stays inside them.
    assert_eq!((timing.length, timing.source), (89, 16));
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
    // A long gap keeps the spine (the timeline) running under all 2,000 items.
    project
        .clips
        .insert(Clip {
            id: ClipId::from_serial(9_999),
            timing: ClipTiming {
                start: 660,
                length: 50_000,
                source: 0,
                speed: 1.0,
            },
            data: ClipData::default(),
        })
        .unwrap();
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
    assert_eq!(project.items.order().len(), 2_000);
    // Best of 5 interleaved passes per sample: preemption on a loaded machine inflates single
    // samples, never all five; a slower apply raises every pass. The threshold is unchanged.
    let mut times = vec![Duration::MAX; 300];
    for _ in 0..5 {
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
                times[i - 6] = times[i - 6].min(at.elapsed());
            }
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

/// T24.3: warm 0.3 edits of existing state (roll, slip, slide, lane order, param.set,
/// transition.set, mask.set) allocate nothing, each op paired with its inverse.
#[test]
fn budget_warm_tier_c_edits_allocate_nothing() {
    let mut project = budget_project();
    let mut workspace = ApplyWorkspace::default();
    let setup = splice_json::parse(
        r#"[{"cmd":"item.trim","item":"clp_end","edge":"in","delta":12},
            {"cmd":"item.trim","item":"clp_take","edge":"in","delta":12},
            {"cmd":"item.trim","item":"clp_take","edge":"out","delta":-12},
            {"cmd":"transition.add","item":"clp_end","kind":"dissolve","length":10},
            {"cmd":"mask.add","item":"clp_take","mode":"blur","x":0.6,"y":0.1,"w":0.2,"h":0.1},
            {"cmd":"param.set","target":"ovl_broll","path":"transform.rotation","value":-4}]"#,
    )
    .unwrap();
    workspace
        .apply(
            &mut project,
            setup.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();
    let slot = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let mask = project
        .clips
        .data(slot)
        .unwrap()
        .appearance
        .effects
        .iter()
        .find(|e| e.kind == "mask")
        .unwrap()
        .id;
    let operations = [
        r#"{"cmd":"item.roll","item":"clp_take","by":-4}"#.to_owned(),
        r#"{"cmd":"item.roll","item":"clp_take","by":4}"#.to_owned(),
        r#"{"cmd":"item.slip","item":"clp_take","by":6}"#.to_owned(),
        r#"{"cmd":"item.slip","item":"clp_take","by":-6}"#.to_owned(),
        r#"{"cmd":"item.slide","item":"clp_take","by":-3}"#.to_owned(),
        r#"{"cmd":"item.slide","item":"clp_take","by":3}"#.to_owned(),
        r#"{"cmd":"lane.reorder","lane":"lan_ov1","to":0}"#.to_owned(),
        r#"{"cmd":"lane.reorder","lane":"lan_ov1","to":3}"#.to_owned(),
        r#"{"cmd":"param.set","target":"ovl_broll","path":"transform.rotation","value":8}"#
            .to_owned(),
        r#"{"cmd":"param.set","target":"ovl_broll","path":"transform.rotation","value":-4}"#
            .to_owned(),
        r#"{"cmd":"transition.set","item":"clp_end","length":14}"#.to_owned(),
        r#"{"cmd":"transition.set","item":"clp_end","length":10}"#.to_owned(),
        format!(r#"{{"cmd":"mask.set","mask":"{mask}","feather":0.2}}"#),
        format!(r#"{{"cmd":"mask.set","mask":"{mask}","feather":0.05}}"#),
    ]
    .map(|op| splice_json::parse(&op).unwrap());
    // Warm every pair (recycled buffers settle at their largest size), then count per op.
    for _ in 0..3 {
        for op in &operations {
            workspace
                .apply(&mut project, std::slice::from_ref(op), Source::Human, false)
                .unwrap();
        }
    }
    let mut counts = Vec::new();
    for op in &operations {
        mem_sys::local_reset();
        workspace
            .apply(&mut project, std::slice::from_ref(op), Source::Human, false)
            .unwrap();
        let cmd = op.field("cmd").and_then(Json::as_str).unwrap();
        counts.push((cmd, mem_sys::local_alloc_count()));
    }
    assert!(counts.iter().all(|(_, n)| *n == 0), "{counts:?}");
}

pub(super) fn run(project: &mut Project, ops: &str) -> Result<ApplyReport, ApplyError> {
    let ops = splice_json::parse(ops).unwrap();
    apply(project, ops.as_array().unwrap(), Source::Agent, false)
}

pub(super) fn start_of(project: &Project, id: &str) -> Frame {
    let id = match id.parse::<ItemId>() {
        Ok(item) => TimelineId::Item(item),
        Err(_) => TimelineId::Clip(id.parse().unwrap()),
    };
    span(project, id).unwrap().start
}

/// The created item of op `index`.
pub(super) fn created(report: &ApplyReport, index: usize) -> String {
    match &report.changes[index].created[0] {
        Target::Timeline(id) => id.to_string(),
        other => panic!("{other:?}"),
    }
}

/// The end of the last item, which must not pass the end of the spine.
pub(super) fn timeline_end(project: &Project) -> (Frame, Frame) {
    let spine = project
        .clips
        .order()
        .iter()
        .map(|&s| project.clips.timing(s).unwrap().end().unwrap())
        .max()
        .unwrap_or(0);
    let items = project
        .items
        .order()
        .iter()
        .map(|&s| {
            span(project, TimelineId::Item(project.items.id(s).unwrap()))
                .unwrap()
                .end
        })
        .max()
        .unwrap_or(0);
    (spine, items)
}

/// T37.2 H1: a clip and an overlay anchored to it, moved together, move once.
#[test]
fn h1_overlay_moved_with_its_clip_moves_once() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"title.add","at":560,"length":30,"text":"End"}]"#,
    )
    .unwrap();
    let title = created(&report, 0);
    run(
        &mut project,
        &format!(
            r#"[{{"cmd":"item.move","items":["clp_end","{title}"],"delta":50,"ripple":false}}]"#
        ),
    )
    .unwrap();
    assert_eq!(start_of(&project, "clp_end"), 590);
    assert_eq!(start_of(&project, &title), 610);
}

/// T37.2 H2: the timeline ends with the spine. A gap keeps an item's frame; past the end is
/// refused; an item running past the end is trimmed.
#[test]
fn h2_gap_and_past_end_placement_keep_the_timeline_at_the_spine() {
    let mut project = sample();
    let error = run(
        &mut project,
        r#"[{"cmd":"title.add","at":670,"length":90,"text":"Late"}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "PAST_END");
    assert!(error.error.rule.is_some());
    let report = run(
        &mut project,
        r#"[{"cmd":"title.add","at":600,"length":90,"text":"Tail"}]"#,
    )
    .unwrap();
    let tail = created(&report, 0);
    assert_eq!(
        span(&project, tail.parse().map(TimelineId::Item).unwrap())
            .unwrap()
            .end,
        660
    );
    assert_eq!(timeline_end(&project), (660, 660));
    // A title placed in a gap stays on its frame instead of snapping to the clip before it.
    run(
        &mut project,
        r#"[{"cmd":"item.delete","items":["clp_take"],"ripple":false}]"#,
    )
    .unwrap();
    let report = run(
        &mut project,
        r#"[{"cmd":"cut.range","from":600,"to":660},{"cmd":"title.add","at":300,"length":30}]"#,
    )
    .unwrap();
    let mid = created(&report, 1);
    assert_eq!(start_of(&project, &mid), 300);
    let (spine, items) = timeline_end(&project);
    assert_eq!(spine, 600);
    assert!(
        items <= spine,
        "items end at {items}, past the spine end {spine}"
    );
}

/// Found by the cursor-range-hide eval: two splits of one clip in a batch. The second frame is
/// past the shortened clip, and the error names the piece that holds it.
#[test]
fn a_second_split_past_the_shortened_clip_names_the_piece_holding_the_frame() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"item.split","item":"clp_take","at":200},{"cmd":"item.split","item":"clp_take","at":300}]"#,
    )
    .unwrap();
    let error = apply(&mut project, ops.as_array().unwrap(), Source::Agent, true).unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("OUT_OF_RANGE") && text.contains("$ref.right"),
        "{text}"
    );
    // The ref form the fix suggests works.
    let ops = splice_json::parse(
        r#"[{"cmd":"item.split","item":"clp_take","at":200,"ref":"a"},{"cmd":"item.split","item":"$a.right","at":300}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, true).unwrap();
    // A stray field in an at object is named as such, with the field it likely meant.
    let ops = splice_json::parse(r#"[{"cmd":"title.add","at":{"clip":"clp_take"},"length":30}]"#)
        .unwrap();
    let error = apply(&mut project, ops.as_array().unwrap(), Source::Agent, true).unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("at has no field clip") && text.contains("\"item\""),
        "{text}"
    );
}

/// Found by record-app-landscape: agents pass clip.crop {id} (it takes items); it is items.
/// Also from the final eval run: $cut.left names a split's left piece, a ref written
/// without $ is pointed at its $ form, and seconds where frames go say so.
#[test]
fn agent_spellings_of_arguments_and_refs_mean_what_they_say() {
    let mut project = sample();
    let run = |project: &mut Project, ops: &str| {
        let ops = splice_json::parse(ops).unwrap();
        apply(project, ops.as_array().unwrap(), Source::Agent, true)
    };
    run(
        &mut project,
        r#"[{"cmd":"clip.crop","id":"clp_take","aspect":"canvas"}]"#,
    )
    .unwrap();
    run(
        &mut project,
        r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"},{"cmd":"cursor.set","clip":"$cut.left","hidden":true}]"#,
    )
    .unwrap();
    let bare = run(
        &mut project,
        r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"},{"cmd":"cursor.set","clip":"cut","hidden":true}]"#,
    )
    .unwrap_err()
    .to_string();
    assert!(bare.contains("\"$cut\""), "{bare}");
    let seconds = run(&mut project, r#"[{"cmd":"cut.pauses","min":0.6}]"#)
        .unwrap_err()
        .to_string();
    assert!(
        seconds.contains("not seconds") && seconds.contains("18 frames"),
        "{seconds}"
    );
}

/// From the final eval run: zoom.set {item} and zoom.remove {items:[…]} (they take zoom).
#[test]
fn zoom_commands_take_the_names_agents_use() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"zoom.add","at":200,"length":60,"amount":1.5,"ref":"z"},
            {"cmd":"zoom.set","item":"$z","amount":2},
            {"cmd":"zoom.remove","items":["$z"]}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, true).unwrap();
}
