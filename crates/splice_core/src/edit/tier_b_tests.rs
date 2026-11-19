use super::tests::sample;
use super::*;
use std::path::Path;

fn run(project: &mut Project, ops: &str) -> Result<ApplyReport, ApplyError> {
    let ops = splice_json::parse(ops).unwrap();
    apply(project, ops.as_array().unwrap(), Source::Agent, false)
}

fn golden(name: &str, project: &Project, report: &ApplyReport) {
    let mut bytes = String::new();
    let value = Json::Object(vec![
        ("project".into(), crate::package::project::encode(project)),
        (
            "mints".into(),
            Json::Array(report.mints.iter().cloned().map(Json::String).collect()),
        ),
    ]);
    splice_json::write(&value, &mut bytes).unwrap();
    bytes.push('\n');
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/json/tier-b")
        .join(format!("{name}.json"));
    splice_testkit::golden(&path, bytes.as_bytes()).unwrap();
}

#[test]
fn tier_b_described_examples_apply_replay_and_match_goldens() {
    let base = sample();
    for command in commands::COMMANDS
        .iter()
        .filter(|row| row.tier == commands::Tier::Second && !row.id.starts_with("caption."))
    {
        let described = commands::describe(Some(command.id), None).unwrap();
        let op = described.field("example").unwrap();
        let mut project = base.clone();
        let report = apply(&mut project, std::slice::from_ref(op), Source::Agent, false)
            .unwrap_or_else(|error| panic!("{}: {error}", command.id));
        assert_eq!(report.changes[0].cmd, command.id);
        let mut replayed = base.clone();
        let again = replay(
            &mut replayed,
            std::slice::from_ref(op),
            Source::Agent,
            &report.mints,
        )
        .unwrap();
        assert_eq!(report, again, "{}", command.id);
        golden(command.id, &project, &report);
    }
}

#[test]
fn tier_b_effects_inherit_override_reset_and_promote() {
    let mut project = sample();
    // Window does not apply to a caption lane; cursor needs a take.
    let error = run(
        &mut project,
        r#"[{"cmd":"effect.add","target":"lan_cap","type":"window"}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "WRONG_KIND");
    let error = run(
        &mut project,
        r#"[{"cmd":"effect.add","target":"clp_open","type":"cursor"}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "WRONG_KIND");
    let error = run(
        &mut project,
        r#"[{"cmd":"effect.add","target":"clp_take","type":"window","params":{"padding":0.9}}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "BAD_ARGS");
    let error = run(
        &mut project,
        r#"[{"cmd":"effect.add","target":"project","type":"spotlight"}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "PLANNED");
    // Set, then reset the only override: the instance disappears.
    run(
        &mut project,
        r#"[{"cmd":"effect.set","target":"clp_end","type":"cursor","params":{}}]"#,
    )
    .unwrap_err();
    run(
        &mut project,
        r#"[{"cmd":"effect.set","target":"clp_take","type":"cursor","params":{"size":3}},
            {"cmd":"effect.set","target":"clp_take","type":"cursor","reset":["size"]}]"#,
    )
    .unwrap();
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let kinds: Vec<_> = project
        .clips
        .data(take)
        .unwrap()
        .appearance
        .effects
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    assert_eq!(kinds, ["window"]);
    // Promote the take's window to the spine lane and clear sibling overrides.
    run(
        &mut project,
        r#"[{"cmd":"effect.promote","effect":"fx_takewin","to":"lane","clearOverrides":true}]"#,
    )
    .unwrap();
    assert!(project.spine.effects.iter().any(|e| e.kind == "window"));
    for &slot in project.clips.order() {
        assert!(
            !project
                .clips
                .data(slot)
                .unwrap()
                .appearance
                .effects
                .iter()
                .any(|e| e.kind == "window")
        );
    }
}

#[test]
fn tier_b_cut_range_splits_ripples_and_keeps_overlays() {
    let mut project = sample();
    let before = time::span(&project, TimelineId::Item("ovl_broll".parse().unwrap())).unwrap();
    run(&mut project, r#"[{"cmd":"cut.range","from":200,"to":260}]"#).unwrap();
    let spans: Vec<_> = project
        .clips
        .order()
        .iter()
        .map(|&slot| {
            let t = project.clips.timing(slot).unwrap();
            (t.start, t.length)
        })
        .collect();
    assert_eq!(spans, [(0, 120), (120, 80), (200, 280), (480, 120)]);
    let after = time::span(&project, TimelineId::Item("ovl_broll".parse().unwrap())).unwrap();
    assert_eq!(after.start, before.start - 60);
    let error = run(&mut project, r#"[{"cmd":"cut.range","from":50,"to":50}]"#).unwrap_err();
    assert_eq!(error.error.code, "BAD_ARGS");
}

#[test]
fn tier_b_components_crop_and_keys() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"component.set","clip":"clp_take","component":"camera","enabled":false},
            {"cmd":"audio.set","clip":"clp_take","component":"mic","volume":3},
            {"cmd":"keys.hide","clip":"clp_take","events":[4,2,4],"hidden":true},
            {"cmd":"clip.crop","items":["clp_take"],"rect":null,"fit":"fill"}]"#,
    )
    .unwrap();
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let data = project.clips.data(take).unwrap();
    assert_eq!(data.enabled.camera, Some(false));
    assert_eq!(data.volume.mic, 2.0);
    assert_eq!(data.hidden_keys, [2, 4]);
    assert_eq!(
        data.framing,
        Framing {
            crop: None,
            fit: Fit::Fill
        }
    );
    for (op, code) in [
        (
            r#"{"cmd":"component.set","clip":"clp_open","component":"mic","enabled":true}"#,
            "WRONG_KIND",
        ),
        (
            r#"{"cmd":"clip.crop","items":["ovl_hook"],"fit":"fit"}"#,
            "WRONG_KIND",
        ),
        (r#"{"cmd":"clip.crop","items":["clp_take"]}"#, "BAD_ARGS"),
        (
            r#"{"cmd":"motion.set","item":"ovl_detail","to":5}"#,
            "BAD_ARGS",
        ),
    ] {
        let error = run(&mut project, &format!("[{op}]")).unwrap_err();
        assert_eq!(error.error.code, code, "{op}");
    }
}

#[test]
fn tier_b_captions_generate_edit_split_merge_and_follow_cuts() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"captions.generate","maxWords":2,"style":"pop","ref":"$gen"}]"#,
    )
    .unwrap();
    let cues = &report.changes[0].created;
    assert!(cues.len() > 10, "{}", cues.len());
    let cue = |project: &Project, target: &commands::Target| {
        let commands::Target::Timeline(TimelineId::Item(id)) = target else {
            panic!()
        };
        let slot = project.items.find(*id).unwrap();
        match &project.items.data(slot).unwrap().kind {
            ItemKind::Caption { words, edits } => (words.clone(), edits.len()),
            _ => panic!(),
        }
    };
    let (words, _) = cue(&project, &cues[0]);
    assert_eq!(words.len(), 2);
    let first = cues[0].clone();
    let second = cues[1].clone();
    let id = |t: &commands::Target| match t {
        commands::Target::Timeline(id) => id.to_string(),
        _ => panic!(),
    };
    run(
        &mut project,
        &format!(
            r#"[{{"cmd":"caption.set","item":"{0}","text":"Hello"}},
                {{"cmd":"caption.merge","items":["{0}","{1}"]}},
                {{"cmd":"caption.split","item":"{0}","word":"{2}"}}]"#,
            id(&first),
            id(&second),
            words[1]
        ),
    )
    .unwrap();
    let (left, edits) = cue(&project, &first);
    assert_eq!(left, words[..1]);
    assert_eq!(edits, 1);
    // Cutting every word of a cue removes it.
    run(
        &mut project,
        &format!(r#"[{{"cmd":"cut.words","words":["{}"]}}]"#, words[0]),
    )
    .unwrap();
    let commands::Target::Timeline(TimelineId::Item(gone)) = first else {
        panic!()
    };
    assert!(project.items.find(gone).is_none());
}

#[test]
fn tier_b_motion_focus_and_ease_apply_validate_and_round_trip() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"motion.set","item":"ovl_detail","focus":{"x":0.25,"y":0.75},"ease":true},
            {"cmd":"motion.add","at":0,"length":30,"to":1.5,"focus":{"x":0.5,"y":0.1}}]"#,
    )
    .unwrap();
    let slot = project.items.find("ovl_detail".parse().unwrap()).unwrap();
    let ItemKind::Motion { focus, ease, .. } = &project.items.data(slot).unwrap().kind else {
        panic!("motion")
    };
    assert_eq!((*focus, *ease), (Some([0.25, 0.75]), true));
    for (op, code) in [
        (
            r#"{"cmd":"motion.set","item":"ovl_detail","focus":{"x":2,"y":0}}"#,
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"motion.add","at":0,"focus":{"x":0.5}}"#,
            "BAD_ARGS",
        ),
    ] {
        let error = run(&mut project, &format!("[{op}]")).unwrap_err();
        assert_eq!(error.error.code, code, "{op}");
    }
    // Saved and reopened, the focus and easing survive; null resets the focus.
    let dir = std::env::temp_dir().join(format!("splice-motion-{}.splice", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    drop(crate::package::Package::create(&dir, &project).unwrap());
    let (package, reopened) = crate::package::Package::open(&dir).unwrap();
    drop(package);
    std::fs::remove_dir_all(&dir).unwrap();
    let slot = reopened.items.find("ovl_detail".parse().unwrap()).unwrap();
    assert_eq!(
        reopened.items.data(slot).unwrap().kind,
        project.items.data(slot).unwrap().kind
    );
    run(
        &mut project,
        r#"[{"cmd":"motion.set","item":"ovl_detail","focus":null}]"#,
    )
    .unwrap();
    let ItemKind::Motion { focus, ease, .. } = &project.items.data(slot).unwrap().kind else {
        panic!("motion")
    };
    assert_eq!((*focus, *ease), (None, true));
}

#[test]
fn tier_b_crop_aspect_reframes_to_fill_without_bands() {
    let mut project = sample();
    // The 640×400 take on the 9:16 canvas: the widest 9:16 slice, around the focus.
    run(
        &mut project,
        r#"[{"cmd":"clip.crop","items":["clp_take"],"aspect":"canvas","focus":{"x":0.9,"y":0.5}}]"#,
    )
    .unwrap();
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let framing = project.clips.data(take).unwrap().framing;
    let crop = framing.crop.unwrap();
    assert_eq!(framing.fit, Fit::Fill);
    assert!(
        (crop.w - 0.5625 / 1.6).abs() < 1e-3 && crop.h == 1.0,
        "{crop:?}"
    );
    assert!(
        (crop.x + crop.w - 1.0).abs() < 1e-3,
        "clamped to the right edge: {crop:?}"
    );
    run(
        &mut project,
        r#"[{"cmd":"clip.crop","items":["clp_take"],"aspect":"1:1"}]"#,
    )
    .unwrap();
    let crop = project.clips.data(take).unwrap().framing.crop.unwrap();
    assert!(
        (crop.w - 0.625).abs() < 1e-3 && (crop.x - 0.1875).abs() < 1e-3,
        "{crop:?}"
    );
    for op in [
        r#"{"cmd":"clip.crop","items":["clp_take"],"aspect":"tall"}"#,
        r#"{"cmd":"clip.crop","items":["clp_take"],"aspect":"1:1","rect":{"x":0,"y":0,"w":1,"h":1}}"#,
        r#"{"cmd":"clip.crop","items":["ovl_hook"],"aspect":"canvas"}"#,
    ] {
        assert!(run(&mut project, &format!("[{op}]")).is_err(), "{op}");
    }
}
