//! T45.4: counters, progress shapes, particles and look packs write ordinary items and effects,
//! and refuse bad arguments naming the rule.
use super::tests::sample;
use super::*;

fn run(project: &mut Project, ops: &str) -> Result<ApplyReport, ApplyError> {
    let ops = splice_json::parse(ops).unwrap();
    apply(project, ops.as_array().unwrap(), Source::Agent, false)
}

fn refused(ops: &str) -> (String, String) {
    let error = run(&mut sample(), ops).unwrap_err().error;
    let param = error.rule.map(|rule| rule.param).unwrap_or_default();
    (error.code.to_owned(), param)
}

fn effect(effects: &[Effect], kind: &str) -> Effect {
    effects
        .iter()
        .find(|effect| effect.kind == kind)
        .cloned()
        .unwrap_or_else(|| panic!("no {kind}"))
}

fn param(effect: &Effect, name: &str) -> EffectValue {
    effect
        .params
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.value.clone())
        .unwrap_or_else(|| panic!("no {name}"))
}

fn created_item(project: &Project, report: &ApplyReport) -> Slot {
    let Some(Target::Timeline(TimelineId::Item(id))) = report.changes[0].created.first() else {
        panic!("{report:?}")
    };
    project.items.find(*id).unwrap()
}

#[test]
fn counter_writes_value_keys_over_the_title_and_checks_its_format() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"counter.add","at":60,"length":30,"from":10,"to":99.5,"decimals":1,"format":"{n}%","ease":"linear"}]"#,
    )
    .unwrap();
    let slot = created_item(&project, &report);
    let data = project.items.data(slot).unwrap();
    assert!(matches!(&data.kind, ItemKind::Title { text } if text == "99.5%"));
    let counter = effect(&data.appearance.effects, "counter");
    let EffectValue::Keys(keys) = param(&counter, "value") else {
        panic!()
    };
    assert_eq!(
        keys.iter().map(|k| (k.at, k.value)).collect::<Vec<_>>(),
        [(0, 10.0), (29, 99.5)]
    );
    assert_eq!(keys[0].ease, Ease::Linear);
    assert_eq!(param(&counter, "decimals"), EffectValue::Number(1.0));
    // The count can be re-keyed like any parameter.
    run(
        &mut project,
        r#"[{"cmd":"param.keys","target":"ovl_5k","path":"counter.value","keys":[{"at":0,"value":5},{"at":10,"value":50}]}]"#,
    )
    .unwrap();
    for (ops, param) in [
        (r#"[{"cmd":"counter.add","at":60}]"#, "to"),
        (
            r#"[{"cmd":"counter.add","at":60,"to":5,"format":"%"}]"#,
            "format",
        ),
        (
            r#"[{"cmd":"counter.add","at":60,"to":5,"decimals":5}]"#,
            "decimals",
        ),
        (r#"[{"cmd":"counter.add","at":60,"to":2e9}]"#, "to"),
        (
            r#"[{"cmd":"counter.add","at":60,"to":5,"in":{"preset":"fade"}}]"#,
            "in",
        ),
    ] {
        assert_eq!(refused(ops).1, param, "{ops}");
    }
}

#[test]
fn progress_runs_to_the_end_of_the_timeline_or_its_length() {
    let mut project = sample();
    let report = run(
        &mut project,
        r##"[{"cmd":"progress.add","fill":"#00ff00","track":"#111111","trackOpacity":0.5}]"##,
    )
    .unwrap();
    let slot = created_item(&project, &report);
    let timing = project.items.timing(slot).unwrap();
    assert_eq!(timing.anchor, Anchor::Pinned(0));
    assert_eq!(timing.length, 660, "the whole timeline");
    let data = project.items.data(slot).unwrap();
    assert!(matches!(
        data.kind,
        ItemKind::Shape {
            shape: ShapeKind::Rect,
            ..
        }
    ));
    let progress = effect(&data.appearance.effects, "progress");
    assert_eq!(param(&progress, "trackOpacity"), EffectValue::Number(0.5));
    let EffectValue::Keys(keys) = param(&progress, "value") else {
        panic!()
    };
    assert_eq!((keys[1].at, keys[1].value), (659, 1.0));
    let shape = effect(&data.appearance.effects, "shape");
    assert_eq!(param(&shape, "corner"), EffectValue::Number(0.5));
    let report = run(
        &mut project,
        r#"[{"cmd":"progress.add","shape":"ring","at":30,"length":90}]"#,
    )
    .unwrap();
    let slot = created_item(&project, &report);
    let data = project.items.data(slot).unwrap();
    let ItemKind::Shape {
        shape: ShapeKind::Ring,
        rect,
    } = data.kind
    else {
        panic!()
    };
    // Circular in pixels on the 1080×1920 canvas.
    assert!((rect.w * 1080.0 - rect.h * 1920.0).abs() < 1e-6);
    assert_eq!(
        param(&effect(&data.appearance.effects, "shape"), "strokeWidth"),
        EffectValue::Number(0.012)
    );
    for (ops, param) in [
        (r#"[{"cmd":"progress.add","shape":"star"}]"#, "shape"),
        (r#"[{"cmd":"progress.add","length":2}]"#, "length"),
        (r#"[{"cmd":"progress.add","length":"forever"}]"#, "length"),
        (
            r#"[{"cmd":"progress.add","trackOpacity":2}]"#,
            "trackOpacity",
        ),
    ] {
        assert_eq!(refused(ops).1, param, "{ops}");
    }
}

#[test]
fn particles_cap_their_count_and_keep_the_seed() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"particles.add","at":90,"preset":"snow","count":120,"seed":42,"rect":{"x":0,"y":0,"w":1,"h":0.5}}]"#,
    )
    .unwrap();
    let slot = created_item(&project, &report);
    let data = project.items.data(slot).unwrap();
    let particles = effect(&data.appearance.effects, "particles");
    assert_eq!(param(&particles, "seed"), EffectValue::Number(42.0));
    assert_eq!(project.items.timing(slot).unwrap().length, 90);
    for (ops, param) in [
        (r#"[{"cmd":"particles.add","at":90,"count":121}]"#, "count"),
        (
            r#"[{"cmd":"particles.add","at":90,"preset":"rain"}]"#,
            "preset",
        ),
        (r#"[{"cmd":"particles.add","at":90,"size":0.5}]"#, "size"),
    ] {
        assert_eq!(refused(ops).1, param, "{ops}");
    }
}

#[test]
fn look_packs_write_ordinary_effects_and_skip_what_does_not_fit() {
    let mut project = sample();
    let before = crate::package::project::encode(&project);
    let report = run(
        &mut project,
        r#"[{"cmd":"look.apply","target":"project","pack":"vivid"},{"cmd":"look.apply","target":"project","pack":"mono"}]"#,
    )
    .unwrap();
    // The later pack replaces every grade parameter the earlier one set.
    let color = effect(&project.effects, "color");
    assert_eq!(param(&color, "saturation"), EffectValue::Number(0.0));
    assert_eq!(param(&color, "vibrance"), EffectValue::Number(0.0));
    assert_eq!(param(&effect(&project.effects, "captions"), "preset"), {
        EffectValue::Text("minimal".into())
    });
    // The project already had a window: it is restyled, not added.
    assert_eq!(
        param(&effect(&project.effects, "window"), "color"),
        EffectValue::Text("#3a3a3a".into())
    );
    assert!(
        report.changes[1]
            .summary
            .contains("color, window, captions")
    );
    // A windowed clip takes the grade and the window colors; captions do not apply to it.
    let report = run(
        &mut project,
        r#"[{"cmd":"look.apply","target":"clp_open","pack":"neon"}]"#,
    )
    .unwrap();
    let summary = &report.changes[0].summary;
    assert!(summary.ends_with("color, window (skipped captions: not on this target)"));
    let slot = project.clips.find("clp_open".parse().unwrap()).unwrap();
    let clip = project.clips.data(slot).unwrap();
    assert_eq!(
        param(&effect(&clip.appearance.effects, "color"), "tint"),
        EffectValue::Number(0.12)
    );
    // An overlay lane has no window of its own: none is added (that would window every
    // picture on it).
    let report = run(
        &mut project,
        r#"[{"cmd":"look.apply","target":"lan_ov1","pack":"neon"}]"#,
    )
    .unwrap();
    let summary = &report.changes[0].summary;
    assert!(summary.contains("skipped window, captions"), "{summary}");
    let lane = project.lanes.iter().find(|l| l.id.to_string() == "lan_ov1");
    assert!(lane.unwrap().effects.iter().all(|e| e.kind != "window"));
    let (code, param) = refused(r#"[{"cmd":"look.apply","target":"project","pack":"noir"}]"#);
    assert_eq!((code.as_str(), param.as_str()), ("BAD_ARGS", "pack"));
    // A refused batch leaves the project as it was.
    let mut untouched = sample();
    run(
        &mut untouched,
        r#"[{"cmd":"look.apply","target":"project","pack":"film"},{"cmd":"look.apply","target":"ovl_nope","pack":"film"}]"#,
    )
    .unwrap_err();
    assert_eq!(crate::package::project::encode(&untouched), before);
}
