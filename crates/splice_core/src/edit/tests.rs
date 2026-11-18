use super::*;
use crate::package::Package;
use std::path::Path;

fn sample() -> Project {
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
    let batch = splice_json::parse(
        r#"[
        {"cmd":"item.split","item":"clp_take","at":372,"ref":"$cut"},
        {"cmd":"overlay.add","kind":"title","at":"$nope"}
    ]"#,
    )
    .unwrap();
    let before = format!("{project:?}");
    let error = apply(
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
    let dry = apply(&mut project, ops, Source::Human, true).unwrap();
    assert_eq!(format!("{project:?}"), before);
    assert!(!dry.diff.is_empty());
    assert_ne!(dry.mints.len(), 0);
    let live = apply(&mut project, ops, Source::Human, false).unwrap();
    assert_eq!(live, dry);

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
