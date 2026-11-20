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

/// T46.3: look.save changes nothing and names what it saves; the file it describes applies
/// back through look.apply {file, effects} (splice mcp fills effects from the file), window and
/// camera included, keys kept.
#[test]
fn saved_looks_round_trip_through_look_apply() {
    let mut styled = sample();
    run(
        &mut styled,
        r##"[{"cmd":"effect.set","target":"project","type":"window","params":{"background":"wallpaper","wallpaper":"ember","padding":0.1}},
            {"cmd":"effect.add","target":"project","type":"color","params":{"saturation":1.3}},
            {"cmd":"effect.set","target":"clp_take","type":"camera","params":{"mirror":true,"aspect":"circle","layout":{"keys":[{"at":0,"value":"bubble"},{"at":90,"value":"hidden"}]}}}]"##,
    )
    .unwrap();
    let before = crate::package::project::encode(&styled);
    let report = run(
        &mut styled,
        r#"[{"cmd":"look.save","target":"project","file":"looks/brand.json"}]"#,
    )
    .unwrap();
    assert_eq!(crate::package::project::encode(&styled), before);
    let summary = &report.changes[0].summary;
    assert!(
        summary.contains("(window, cursor, color)") && summary.contains("looks/brand.json"),
        "{summary}"
    );
    let file = saved_look(&styled, &Target::Project, std::path::Path::new("/pkg")).unwrap();
    assert_eq!(file.field("splice").and_then(Json::as_str), Some("look"));
    let window = file
        .field("effects")
        .and_then(|e| e.field("window"))
        .unwrap();
    assert_eq!(
        window.field("wallpaper").and_then(Json::as_str),
        Some("ember")
    );
    let take = Target::Timeline(TimelineId::Clip("clp_take".parse().unwrap()));
    let camera = saved_look(&styled, &take, std::path::Path::new("/pkg")).unwrap();
    let mut text = String::new();
    splice_json::write(camera.field("effects").unwrap(), &mut text).unwrap();
    // A fresh project gets the same values from the file.
    let mut fresh = sample();
    run(
        &mut fresh,
        &format!(r#"[{{"cmd":"look.apply","target":"clp_take","file":"looks/cam.json","effects":{text}}}]"#),
    )
    .unwrap();
    let slot = fresh.clips.find("clp_take".parse().unwrap()).unwrap();
    let applied = effect(
        &fresh.clips.data(slot).unwrap().appearance.effects,
        "camera",
    );
    assert_eq!(param(&applied, "mirror"), EffectValue::Bool(true));
    assert_eq!(
        param(&applied, "aspect"),
        EffectValue::Text("circle".into())
    );
    let EffectValue::Keys(keys) = param(&applied, "layout") else {
        panic!("layout keys")
    };
    assert_eq!(
        keys.iter().map(|k| (k.at, k.value)).collect::<Vec<_>>(),
        [(0, 0.0), (90, 2.0)]
    );
    // Nothing to save, an unknown look type and a missing fill are refused with the rule.
    assert_eq!(
        refused(r#"[{"cmd":"look.save","target":"clp_open","file":"a.json"}]"#),
        ("NOTHING_TO_SAVE".into(), "target".into())
    );
    assert_eq!(
        refused(
            r#"[{"cmd":"look.apply","target":"project","file":"a.json","effects":{"shape":{}}}]"#
        )
        .0,
        "UNKNOWN_PARAM"
    );
    assert_eq!(
        refused(r#"[{"cmd":"look.apply","target":"project","file":"a.json"}]"#),
        ("BAD_ARGS".into(), "effects".into())
    );
}

/// T46.3: a window image given as a path imports once like any file (the journal keeps the
/// descriptor) and is stored as its media ID.
#[test]
fn window_images_import_by_path_and_store_the_media_id() {
    let base = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"effect.set","target":"clp_take","type":"window","params":{"background":"image","image":"/tmp/wall.png","blur":0.3}},
            {"cmd":"effect.set","target":"clp_open","type":"window","params":{"background":"image","image":"/tmp/wall.png"}}]"#,
    )
    .unwrap();
    let mut loads = 0;
    let mut load = |path: &str| {
        loads += 1;
        Ok(Media {
            id: MediaId::from_serial(0),
            kind: MediaKind::Image,
            path: path.into(),
            bookmark: None,
            size: 1,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 1_000_000,
                width: 64,
                height: 64,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        })
    };
    let (project, _, imports) = stage_above(
        &base,
        ops.as_array().unwrap(),
        Source::Agent,
        None,
        base.serial,
        &mut load,
    )
    .unwrap();
    assert_eq!((loads, imports.len()), (1, 1));
    let id = imports[0].descriptor.id;
    let slot = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let window = effect(
        &project.clips.data(slot).unwrap().appearance.effects,
        "window",
    );
    assert_eq!(param(&window, "image"), EffectValue::Text(id.to_string()));
    // Without a media backend a path is offline; a non-image media ID is the wrong kind.
    assert_eq!(
        refused(r#"[{"cmd":"effect.set","target":"clp_take","type":"window","params":{"image":"/tmp/wall.png"}}]"#).0,
        "MEDIA_OFFLINE"
    );
    assert_eq!(
        refused(
            r#"[{"cmd":"effect.set","target":"clp_take","type":"window","params":{"image":"med_broll"}}]"#
        ),
        ("WRONG_KIND".into(), "image".into())
    );
}

/// T46.3: mask.add/mask.set from/to are timeline frames inside the owner, stored on its clock.
#[test]
fn timed_masks_refuse_frames_outside_their_item() {
    // clp_take covers 120–540.
    assert_eq!(
        refused(r#"[{"cmd":"mask.add","item":"clp_take","from":100,"to":200}]"#),
        ("OUT_OF_RANGE".into(), "from".into())
    );
    assert_eq!(
        refused(r#"[{"cmd":"mask.add","item":"clp_take","from":200,"to":541}]"#),
        ("OUT_OF_RANGE".into(), "to".into())
    );
    assert_eq!(
        refused(r#"[{"cmd":"mask.add","item":"clp_take","from":200,"to":200}]"#),
        ("BAD_ARGS".into(), "to".into())
    );
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"mask.add","item":"clp_take","from":200,"ref":"m"},{"cmd":"mask.set","mask":"$m","to":300}]"#,
    )
    .unwrap();
    let slot = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let mask = effect(
        &project.clips.data(slot).unwrap().appearance.effects,
        "mask",
    );
    assert_eq!(
        (param(&mask, "start"), param(&mask, "end")),
        (EffectValue::Number(80.0), EffectValue::Number(180.0))
    );
}
