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
