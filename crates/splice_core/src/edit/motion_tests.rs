//! T42: eases, inline title looks, paint order, motion presets and templates.
use super::tests::sample;
use super::*;

fn run(project: &mut Project, ops: &str) -> Result<ApplyReport, ApplyError> {
    let ops = splice_json::parse(ops).unwrap();
    apply(project, ops.as_array().unwrap(), Source::Agent, false)
}

fn effect(project: &Project, item: &str, kind: &str) -> Option<Effect> {
    let slot = project.items.find(item.parse().unwrap()).unwrap();
    project
        .items
        .data(slot)
        .unwrap()
        .appearance
        .effects
        .iter()
        .find(|effect| effect.kind == kind)
        .cloned()
}

fn keys(project: &Project, item: &str, kind: &str, name: &str) -> Vec<Key> {
    let effect = effect(project, item, kind).unwrap();
    match &effect.params.iter().find(|p| p.name == name).unwrap().value {
        EffectValue::Keys(keys) => keys.clone(),
        other => panic!("{other:?}"),
    }
}

/// T42.1: springs, back, elastic, bounce and steps parse, survive the file format, report
/// their settle time, and a bad ease names the key and every accepted form.
#[test]
fn eases_parse_round_trip_and_report_spring_settle() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"param.keys","target":"ovl_hook","path":"transform.scale","keys":[
            {"at":0,"value":0.5,"ease":{"spring":{"duration":20,"bounce":0.3}}},
            {"at":40,"value":1,"ease":"back"},
            {"at":50,"value":1.2,"ease":{"steps":4}},
            {"at":60,"value":1,"ease":"elastic"},
            {"at":70,"value":1.1,"ease":"bounce"},
            {"at":80,"value":1}]}]"#,
    )
    .unwrap();
    let detail = report.changes[0].detail.unwrap();
    let expected = Ease::Spring {
        duration: 20.0,
        bounce: 0.3,
    }
    .settle()
    .unwrap();
    assert_eq!(detail.settle, Some(expected));
    assert!(expected < 40.0, "{expected}");
    assert!(!report.changes[0].summary.contains("jumps"));
    let written = keys(&project, "ovl_hook", "transform", "scale");
    assert_eq!(
        written.iter().map(|k| k.ease).collect::<Vec<_>>(),
        [
            Ease::Spring {
                duration: 20.0,
                bounce: 0.3
            },
            Ease::Back,
            Ease::Steps(4),
            Ease::Elastic,
            Ease::Bounce,
            Ease::InOut,
        ]
    );
    let encoded = crate::package::encode_keys(&written);
    assert_eq!(crate::package::decode_keys(&encoded).unwrap(), written);
    let mut text = String::new();
    splice_json::write(&encoded, &mut text).unwrap();
    assert!(
        text.contains(r#""ease":{"spring":{"duration":20,"bounce":0.3}}"#),
        "{text}"
    );
    assert!(text.contains(r#""ease":{"steps":4}"#), "{text}");

    // A spring cut short by the next key says so.
    let report = run(
        &mut project,
        r#"[{"cmd":"param.keys","target":"ovl_hook","path":"transform.y","keys":[
            {"at":0,"value":0.1,"ease":{"spring":{"duration":30}}},{"at":10,"value":0}]}]"#,
    )
    .unwrap();
    assert!(
        report.changes[0]
            .summary
            .contains("needs 45 frames to settle"),
        "{}",
        report.changes[0].summary
    );

    for bad in [
        r#""wobble""#,
        r#"{"steps":0}"#,
        r#"{"spring":{"duration":1}}"#,
        r#"{"spring":{"duration":30,"bounce":1}}"#,
        r#"{"spring":{"duration":30,"stiffness":100}}"#,
    ] {
        let error = run(
            &mut project,
            &format!(
                r#"[{{"cmd":"param.keys","target":"ovl_hook","path":"transform.x","keys":[
                    {{"at":0,"value":0}},{{"at":10,"value":0.1,"ease":{bad}}}]}}]"#
            ),
        )
        .unwrap_err();
        let rule = error.error.rule.unwrap();
        assert_eq!(rule.param, "keys[1].ease", "{bad}");
        assert!(rule.expected.contains("{spring: {duration"), "{bad}");
        assert_eq!(rule.got, Some(splice_json::parse(bad).unwrap()));
    }
}

/// T42.2: title.add compiles box, style, in, out and loop to layout/text/animate effects and
/// transform keys, returns its final and swept bounds, and refuses a box off the canvas.
#[test]
fn title_inline_look_compiles_to_effects_and_reports_bounds() {
    let mut project = sample();
    let report = run(
        &mut project,
        r##"[{"cmd":"title.add","at":30,"length":90,"text":"cat ➜ DRAGON 🐉","ref":"t",
              "box":{"x":0.1,"y":0.1,"w":0.8,"h":0.2,"fit":"shrink","wrap":true},
              "style":{"size":0.09,"color":"#ffd23f","strokeWidth":0.1},
              "in":{"preset":"pop","unit":"character","stagger":2},
              "out":{"preset":"sink","duration":10},
              "loop":{"preset":"pulse","period":30,"intensity":0.5}}]"##,
    )
    .unwrap();
    let id = match &report.changes[0].created[0] {
        Target::Timeline(TimelineId::Item(id)) => id.to_string(),
        other => panic!("{other:?}"),
    };
    let layout = effect(&project, &id, "layout").unwrap();
    assert_eq!(layout.params.len(), 6);
    let text = effect(&project, &id, "text").unwrap();
    assert!(
        text.params
            .iter()
            .any(|p| p.name == "color" && p.value == EffectValue::Text("#ffd23f".into()))
    );
    let animate = effect(&project, &id, "animate").unwrap();
    assert!(
        animate
            .params
            .iter()
            .any(|p| p.name == "unit" && p.value == EffectValue::Text("character".into()))
    );
    // loop: pulse every 15 frames until the exit; out: sink over the last 10 frames.
    let scale = keys(&project, &id, "transform", "scale");
    assert_eq!(scale.first().unwrap().at, 0);
    assert_eq!(scale.last().unwrap().at, 80);
    assert!(scale.iter().any(|k| (k.value - 1.05).abs() < 1e-9));
    let y = keys(&project, &id, "transform", "y");
    assert_eq!(
        (y[0].at, y[1].at, y[0].ease, y[1].value),
        (80, 90, Ease::In, 0.14)
    );
    let opacity = keys(&project, &id, "transform", "opacity");
    assert_eq!(opacity.last().unwrap().value, 0.0);
    // Final: the box at rest; swept: grown by the pulse (scale 1.05) and the sink below it.
    let [end, swept] = report.changes[0].detail.unwrap().bounds.unwrap();
    let close = |a: [f64; 4], b: [f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
    assert!(close(end, [0.1, 0.1, 0.8, 0.2]), "{end:?}");
    assert!(swept[0] < 0.1 - 0.015 && swept[2] > 0.8 * 1.04, "{swept:?}");
    assert!(swept[1] + swept[3] > 0.3 + 0.1, "{swept:?}");

    // title.set merges the box and reports the new bounds.
    let report = run(
        &mut project,
        &format!(r#"[{{"cmd":"title.set","item":"{id}","box":{{"y":0.5}}}}]"#),
    )
    .unwrap();
    let [end, _] = report.changes[0].detail.unwrap().bounds.unwrap();
    assert!(close(end, [0.1, 0.5, 0.8, 0.2]), "{end:?}");
    // A plain title reports the default band it draws in.
    let report = run(&mut project, r#"[{"cmd":"title.add","at":0,"text":"Hi"}]"#).unwrap();
    let [end, swept] = report.changes[0].detail.unwrap().bounds.unwrap();
    assert!(close(end, swept), "{end:?} {swept:?}");
    assert!((end[0] - 0.06).abs() < 1e-9 && (end[1] - 0.07).abs() < 1e-9);

    for (op, param, code) in [
        (
            r#"{"cmd":"title.set","item":"ovl_hook","box":{"x":0.5,"w":0.6}}"#,
            "box",
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"title.set","item":"ovl_hook","box":{"wdth":0.6}}"#,
            "box.wdth",
            "UNKNOWN_PARAM",
        ),
        (
            r#"{"cmd":"title.set","item":"ovl_hook","style":{"size":2}}"#,
            "style.size",
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"title.set","item":"ovl_hook","out":{"preset":"explode"}}"#,
            "out.preset",
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"title.set","item":"ovl_hook","out":{"preset":"fade","duration":120}}"#,
            "out.duration",
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"title.set","item":"ovl_hook","loop":{"preset":"float","period":2}}"#,
            "loop.period",
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"title.set","item":"ovl_hook"}"#,
            "text",
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"title.set","item":"ovl_broll","text":"x"}"#,
            "",
            "WRONG_KIND",
        ),
    ] {
        let error = run(&mut project, &format!("[{op}]")).unwrap_err().error;
        assert_eq!(error.code, code, "{op}: {error}");
        if !param.is_empty() {
            assert_eq!(error.rule.unwrap().param, param, "{op}");
        }
    }
    let error = run(
        &mut project,
        r#"[{"cmd":"title.set","item":"ovl_hook","box":{"fitt":"fill"}}]"#,
    )
    .unwrap_err()
    .error;
    assert_eq!(error.rule.unwrap().did_you_mean.as_deref(), Some("fit"));
}

/// T42.4: item.order sets z (higher = front), saved in the file; it refuses what does not
/// paint.
#[test]
fn item_order_sets_z_and_refuses_what_does_not_paint() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"item.order","item":"ovl_hook","to":"front"},
            {"cmd":"item.order","item":"ovl_logo","to":{"below":"ovl_hook"}},
            {"cmd":"item.order","item":"ovl_cap1","to":-3},
            {"cmd":"item.order","item":"ovl_broll","to":"back"}]"#,
    )
    .unwrap();
    let z: Vec<_> = report.changes.iter().map(|c| c.detail.unwrap().z).collect();
    assert_eq!(z, [Some(1), Some(0), Some(-3), Some(-4)]);
    let mut text = String::new();
    splice_json::write(&crate::package::project::encode(&project), &mut text).unwrap();
    assert!(text.contains(r#""z":1"#) && text.contains(r#""z":-4"#));
    for (op, code) in [
        (
            r#"{"cmd":"item.order","item":"ovl_detail","to":"front"}"#,
            "WRONG_KIND",
        ),
        (
            r#"{"cmd":"item.order","item":"clp_take","to":"front"}"#,
            "WRONG_KIND",
        ),
        (
            r#"{"cmd":"item.order","item":"ovl_hook","to":"top"}"#,
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"item.order","item":"ovl_hook","to":{"above":"ovl_hook"}}"#,
            "BAD_ARGS",
        ),
    ] {
        assert_eq!(
            run(&mut project, &format!("[{op}]"))
                .unwrap_err()
                .error
                .code,
            code,
            "{op}"
        );
    }
}

/// T42.5: motion.preset writes ordinary transform keys (editable with param.keys), keeps keys
/// outside its window, moves from the item's own values, and bounceIn's spring settles inside
/// its length.
#[test]
fn motion_presets_write_ordinary_keys() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"param.set","target":"ovl_broll","path":"transform.scale","value":0.8},
            {"cmd":"param.keys","target":"ovl_hook","path":"transform.x","keys":[
                {"at":100,"value":0.1},{"at":110,"value":0}]}]"#,
    )
    .unwrap();
    let report = run(
        &mut project,
        r#"[{"cmd":"motion.preset","items":["ovl_hook","ovl_broll"],"preset":"bounceIn","length":20},
            {"cmd":"motion.preset","items":"ovl_hook","preset":"shake","at":40,"intensity":1},
            {"cmd":"motion.preset","items":"ovl_broll","preset":"pulse","at":30},
            {"cmd":"motion.preset","items":"clp_take","preset":"wiggle","length":40}]"#,
    )
    .unwrap();
    let settle = report.changes[0].detail.unwrap().settle.unwrap();
    assert!(settle <= 20.0, "{settle}");
    assert!(
        report.changes[0]
            .summary
            .starts_with("Added bounceIn to 2 item(s)")
    );
    let scale = keys(&project, "ovl_broll", "transform", "scale");
    // bounceIn from the overlay's own 0.8, then pulse from frame 30 to its end (75).
    assert!((scale[0].value - 0.8 * 0.6).abs() < 1e-9);
    assert!(matches!(scale[0].ease, Ease::Spring { .. }));
    assert_eq!((scale[1].at, scale[1].value), (20, 0.8));
    assert_eq!(scale.last().unwrap().at, 75);
    assert!(scale.iter().any(|k| (k.value - 0.8 * 1.05).abs() < 1e-9));
    // shake moves from the held 0.1 and replaced nothing outside [40, 58]: the keys at 100
    // and 110 stay.
    let x = keys(&project, "ovl_hook", "transform", "x");
    assert_eq!((x[0].at, x[0].value), (40, 0.1));
    assert!(x.iter().any(|k| k.at == 42 && k.value < 0.1 - 0.025));
    assert_eq!(x.iter().filter(|k| k.at >= 100).count(), 2);
    // Spine clips take presets too.
    let slot = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let take = &project.clips.data(slot).unwrap().appearance.effects;
    assert!(take.iter().any(|e| e.kind == "transform"));

    for (op, code) in [
        (
            r#"{"cmd":"motion.preset","items":"ovl_hook","preset":"spin"}"#,
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"motion.preset","items":"ovl_hook","preset":"pop","at":115}"#,
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"motion.preset","items":"ovl_detail","preset":"pop"}"#,
            "WRONG_KIND",
        ),
        (
            r#"{"cmd":"motion.preset","items":"ovl_hook","preset":"pop","intensity":2}"#,
            "BAD_ARGS",
        ),
    ] {
        assert_eq!(
            run(&mut project, &format!("[{op}]"))
                .unwrap_err()
                .error
                .code,
            code,
            "{op}"
        );
    }
}

/// T42.6: templates fill with the brand kit, keep text readable on light fills, and sit inside
/// the safe zone (TikTok's on portrait, 5% on landscape), text boxes included.
#[test]
fn templates_use_brand_colors_and_safe_zones() {
    let text_param = |project: &Project, item: &str, kind: &str, name: &str| {
        let effect = effect(project, item, kind).unwrap();
        effect
            .params
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.value.clone())
            .unwrap()
    };
    let members = |report: &ApplyReport| -> Vec<String> {
        report.changes[report.changes.len() - 1]
            .created
            .iter()
            .map(|target| match target {
                Target::Timeline(id) => id.to_string(),
                other => panic!("{other:?}"),
            })
            .collect()
    };
    let rect = |project: &Project, item: &str| {
        let slot = project.items.find(item.parse().unwrap()).unwrap();
        match &project.items.data(slot).unwrap().kind {
            ItemKind::Shape { rect, .. } => [rect.x, rect.y, rect.w, rect.h],
            other => panic!("{other:?}"),
        }
    };
    let text_box = |project: &Project, item: &str| {
        ["x", "y", "w", "h"].map(|name| match text_param(project, item, "layout", name) {
            EffectValue::Number(n) => n,
            other => panic!("{other:?}"),
        })
    };
    // Portrait (the sample is 1080×1920): TikTok's zone keeps 20% clear at the bottom.
    let mut project = sample();
    let report = run(
        &mut project,
        r##"[{"cmd":"style.set","color":"#f4f1ea","color2":"#14213d","accent":"#fca311","textColor":"#ffffff"},
            {"cmd":"template.add","name":"cta","at":150,"fields":{"title":"Follow"}}]"##,
    )
    .unwrap();
    let ids = members(&report);
    let panel = rect(&project, &ids[0]);
    assert!(
        panel[1] + panel[3] <= 0.8 + 1e-9 && panel[1] >= 0.1,
        "{panel:?}"
    );
    assert_eq!(
        text_param(&project, &ids[0], "shape", "fill"),
        EffectValue::Text("#fca311".into())
    );
    // White text on the light accent would vanish: the brand's dark swatch reads instead.
    assert_eq!(
        text_param(&project, &ids[1], "text", "color"),
        EffectValue::Text("#14213d".into())
    );
    let [x, y, w, h] = text_box(&project, &ids[1]);
    assert!(x >= panel[0] && x + w <= panel[0] + panel[2] + 1e-9);
    assert!(y >= panel[1] && y + h <= panel[1] + panel[3] + 1e-9);
    // A lower third on the dark secondary keeps the brand text color.
    let report = run(
        &mut project,
        r#"[{"cmd":"template.add","name":"lower-third","at":300,"fields":{"title":"Ada"}}]"#,
    )
    .unwrap();
    let ids = members(&report);
    assert_eq!(
        text_param(&project, &ids[0], "shape", "fill"),
        EffectValue::Text("#14213d".into())
    );
    assert_eq!(
        text_param(&project, &ids[1], "text", "color"),
        EffectValue::Text("#ffffff".into())
    );
    // Landscape: 5% margins; a full-canvas card keeps its text inside them.
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"project.set","aspect":"16:9"},
            {"cmd":"template.add","name":"title-card","at":0,"fields":{"title":"Intro"}}]"#,
    )
    .unwrap();
    let ids = members(&report);
    assert_eq!(rect(&project, &ids[0]), [0.0, 0.0, 1.0, 1.0]);
    let [x, _, w, _] = text_box(&project, &ids[1]);
    assert!((x - 0.05).abs() < 1e-9 && (w - 0.9).abs() < 1e-9);
    // Without a brand kit, the brand effect's defaults (not black).
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"template.add","name":"chapter-card","at":0}]"#,
    )
    .unwrap();
    let ids = members(&report);
    assert_eq!(
        text_param(&project, &ids[0], "shape", "fill2"),
        EffectValue::Text("#1d2420".into())
    );
}

#[test]
fn v13_scale_motion_defaults_and_validation_follow_the_timeline_rules() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"motion.add","at":{"item":"clp_take","offset":30},"from":null}]"#,
    )
    .unwrap();
    let Target::Timeline(TimelineId::Item(id)) = report.changes[0].created[0] else {
        panic!("expected an overlay ID");
    };
    let slot = project.items.find(id).unwrap();
    let data = project.items.data(slot).unwrap();
    assert_eq!(data.kind, ItemKind::Motion(ScaleMotion::default()));
    assert_eq!(project.items.timing(slot).unwrap().length, 120);
    assert_eq!(time::item_span(&project, slot).unwrap().start, 150);
    assert_eq!(data.kind.lane_kind(), LaneKind::Camera);

    for (ops, expected) in [
        (r#"[{"cmd":"motion.add","at":20,"length":2}]"#, "TOO_SHORT"),
        (r#"[{"cmd":"motion.add","at":20,"length":4.5}]"#, "BAD_ARGS"),
        (r#"[{"cmd":"motion.add","at":20,"from":"bad"}]"#, "BAD_ARGS"),
        (
            r#"[{"cmd":"motion.add","at":150,"lane":"lan_gfx"}]"#,
            "WRONG_KIND",
        ),
        (
            r#"[{"cmd":"motion.add","at":180,"lane":"lan_mo"}]"#,
            "LANE_CONFLICT",
        ),
        (
            r#"[{"cmd":"motion.set","item":"ovl_detail","to":1.8}]"#,
            "WRONG_KIND",
        ),
    ] {
        let error = run(&mut project, ops).unwrap_err();
        assert_eq!(error.error.code, expected, "{error}");
        assert_eq!(
            project.items.data(slot).unwrap().kind,
            ItemKind::Motion(ScaleMotion::default())
        );
    }

    let edits = [
        format!(r#"{{"cmd":"motion.set","item":"{id}","from":2.2}}"#),
        format!(r#"{{"cmd":"motion.set","item":"{id}","to":0.2}}"#),
        format!(r#"{{"cmd":"motion.set","item":"{id}","from":0.2}}"#),
        format!(r#"{{"cmd":"motion.set","item":"{id}","to":7}}"#),
    ]
    .map(|edit| splice_json::parse(&edit).unwrap());
    let mut workspace = ApplyWorkspace::default();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for edit in &edits {
                workspace
                    .apply(
                        &mut project,
                        std::slice::from_ref(edit),
                        Source::Human,
                        false,
                    )
                    .unwrap();
            }
        },
    );
    assert_eq!(
        project.items.data(slot).unwrap().kind,
        ItemKind::Motion(ScaleMotion { from: 0.5, to: 2.0 })
    );
}
