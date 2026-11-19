//! 0.3 rows: every described example applies (after a per-row setup), replays with the same
//! mints, undoes to the base hash, and matches its golden.
use super::tests::sample;
use super::*;
use std::path::Path;

/// Ops that make a row's example applicable to the sample.
fn setup(id: &str) -> &'static str {
    match id {
        "clip.join" => r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"}]"#,
        "clip.ungroup" => {
            r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"},{"cmd":"clip.join","items":["clp_take","$cut.right"]}]"#
        }
        "clip.restore" => r#"[{"cmd":"cut.range","from":200,"to":230}]"#,
        "mask.set" | "mask.track" => r#"[{"cmd":"mask.add","item":"clp_take","ref":"mask"}]"#,
        "audio.link" | "audio.sync" => {
            r#"[{"cmd":"audio.unlink","item":"clp_take","ref":"voice"}]"#
        }
        "clip.uncompound" => {
            r#"[{"cmd":"clip.compound","items":["clp_take","clp_end"],"ref":"compound"}]"#
        }
        "transition.set" | "transition.remove" => r#"[{"cmd":"transition.add","item":"clp_end"}]"#,
        "template.set" => r#"[{"cmd":"template.add","name":"lower-third","at":150,"ref":"lower"}]"#,
        "item.slide" => r#"[{"cmd":"item.trim","item":"clp_end","edge":"in","delta":12}]"#,
        _ => "[]",
    }
}

fn golden(name: &str, project: &Project) {
    let mut bytes = String::new();
    splice_json::write(&crate::package::project::encode(project), &mut bytes).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/json/tier-c")
        .join(format!("{name}.json"));
    splice_testkit::golden(&path, bytes.as_bytes()).unwrap();
}

#[test]
fn tier_c_described_examples_apply_replay_undo_and_match_goldens() {
    let sample = sample();
    let rows: Vec<_> = commands::COMMANDS
        .iter()
        .filter(|row| row.tier == commands::Tier::Third)
        .collect();
    assert_eq!(rows.len(), 35);
    for command in rows {
        let described = commands::describe(Some(command.id), None).unwrap();
        let example = described.field("example").unwrap().clone();
        let mut ops = splice_json::parse(setup(command.id)).unwrap();
        let Json::Array(list) = &mut ops else {
            panic!()
        };
        list.push(example);
        let mut project = sample.clone();
        let report = apply(&mut project, list, Source::Agent, false)
            .unwrap_or_else(|error| panic!("{}: {error}", command.id));
        assert_eq!(report.changes.last().unwrap().cmd, command.id);
        let mut replayed = sample.clone();
        let again = replay(&mut replayed, list, Source::Agent, &report.mints).unwrap();
        assert_eq!(report, again, "{}", command.id);
        assert_eq!(
            crate::package::project::encode(&project),
            crate::package::project::encode(&replayed),
            "{}",
            command.id
        );
        // Round trip through the package format.
        let mut bytes = String::new();
        splice_json::write(&crate::package::project::encode(&project), &mut bytes).unwrap();
        let decoded =
            crate::package::project::decode(&splice_json::parse(&bytes).unwrap()).unwrap();
        let mut again = String::new();
        splice_json::write(&crate::package::project::encode(&decoded), &mut again).unwrap();
        assert_eq!(bytes, again, "{}", command.id);
        golden(command.id, &project);
    }
}

#[test]
fn baked_ducking_becomes_visible_volume_keys() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"audio.unlink","item":"clp_take","ref":"voice"},
            {"cmd":"audio.set","items":["$voice"],"duck":{"amount":0.3,"attack":6,"release":10,"bake":true}}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let slot = *project.items.order().last().unwrap();
    let data = project.items.data(slot).unwrap();
    let ItemKind::Sound(sound) = &data.kind else {
        panic!()
    };
    assert!(sound.duck.is_none());
    let volume = data
        .appearance
        .effects
        .iter()
        .find(|e| e.kind == "volume")
        .unwrap();
    let EffectValue::Keys(keys) = &volume.params[0].value else {
        panic!()
    };
    assert!(keys.len() >= 5, "{keys:?}");
    assert_eq!(keys[0].value, 1.0);
    assert!(keys.iter().any(|k| k.value == 0.3));
    assert!(keys.windows(2).all(|w| w[0].at < w[1].at));
}

/// Scale at timeline frame `at`, from whichever spine clip covers it (owner-relative keys).
fn scale_at(project: &Project, at: Frame) -> f64 {
    for &slot in project.clips.order() {
        let timing = project.clips.timing(slot).unwrap();
        if timing.start <= at && at < timing.start + timing.length {
            let data = project.clips.data(slot).unwrap();
            let Some(param) = data
                .appearance
                .effects
                .iter()
                .find(|e| e.kind == "transform")
                .and_then(|e| e.params.iter().find(|p| p.name == "scale"))
            else {
                return 1.0;
            };
            let EffectValue::Keys(keys) = &param.value else {
                panic!("{param:?}")
            };
            return splice_model::keys::sample(keys, f64::from(at - timing.start));
        }
    }
    panic!("no clip at {at}")
}

#[test]
fn keys_stay_on_their_content_across_split_trim_and_roll() {
    let mut project = sample();
    // Source handles on both sides of the opener/take cut: a transition consumes them (D14).
    let opener = "med_opener".parse().unwrap();
    project
        .media
        .iter_mut()
        .find(|m| m.id == opener)
        .unwrap()
        .probe
        .frames = 150;
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    project.clips.timing_mut(take).unwrap().source = 30;
    // clp_take covers 120..540; zoom 1 → 2 over its first 200 frames.
    let ops = splice_json::parse(
        r#"[{"cmd":"param.keys","target":"clp_take","path":"transform.scale","keys":[{"at":0,"value":1},{"at":200,"value":2}]},
            {"cmd":"transition.add","item":"clp_take","kind":"dissolve","length":10}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let before: Vec<f64> = [150, 250, 300]
        .iter()
        .map(|&at| scale_at(&project, at))
        .collect();
    let edits =
        splice_json::parse(r#"[{"cmd":"item.split","item":"clp_take","at":220,"ref":"cut"}]"#)
            .unwrap();
    apply(
        &mut project,
        edits.as_array().unwrap(),
        Source::Agent,
        false,
    )
    .unwrap();
    let after: Vec<f64> = [150, 250, 300]
        .iter()
        .map(|&at| scale_at(&project, at))
        .collect();
    assert_eq!(before, after, "split moved the zoom");
    // Only the left piece keeps the incoming dissolve.
    let transitions = project
        .clips
        .order()
        .iter()
        .filter(|&&slot| {
            let data = project.clips.data(slot).unwrap();
            data.appearance
                .effects
                .iter()
                .any(|e| e.kind == "transition")
        })
        .count();
    assert_eq!(transitions, 1);
    // Trimming the right piece's in-edge by 20 frames (no ripple) keeps frame 300's zoom.
    let id = project
        .clips
        .order()
        .iter()
        .find(|&&slot| project.clips.timing(slot).unwrap().start == 220)
        .map(|&slot| project.clips.id(slot).unwrap())
        .unwrap();
    let trim = splice_json::parse(&format!(
        r#"[{{"cmd":"item.trim","item":"{id}","edge":"in","delta":20,"ripple":false}}]"#
    ))
    .unwrap();
    apply(&mut project, trim.as_array().unwrap(), Source::Agent, false).unwrap();
    assert!((scale_at(&project, 300) - before[2]).abs() < 1e-9);
}

/// Rows that decode an effect schema accept every parameter of it (no BAD_ARGS for a schema
/// parameter that `effect.add` would take).
#[test]
fn schema_backed_rows_list_every_schema_parameter() {
    let schemas = commands::effect_schemas();
    for (id, schema, skip) in [
        ("mask.add", "mask", &[][..]),
        ("mask.set", "mask", &[]),
        ("transition.add", "transition", &[]),
        ("transition.set", "transition", &[]),
        ("style.set", "brand", &[]),
        ("generator.add", "shape", &[]),
    ] {
        let row = commands::COMMANDS.iter().find(|row| row.id == id).unwrap();
        let Some(Json::Object(params)) = schemas.field(schema).and_then(|s| s.field("params"))
        else {
            panic!("{schema}")
        };
        let missing: Vec<&str> = params
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| !skip.contains(name) && !row.params.iter().any(|(p, _)| p == name))
            .collect();
        assert!(missing.is_empty(), "{id} lacks {missing:?}");
    }
}

#[test]
fn an_inserted_still_lasts_five_seconds() {
    let mut project = sample();
    let ops = splice_json::parse(r#"[{"cmd":"clip.insert","media":"med_logo","at":120}]"#).unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let still = project
        .clips
        .order()
        .iter()
        .find(|&&slot| project.clips.data(slot).unwrap().media == Some("med_logo".parse().unwrap()))
        .map(|&slot| project.clips.timing(slot).unwrap())
        .unwrap();
    assert_eq!((still.start, still.length), (120, 5 * project.fps as Frame));
}

#[test]
fn compounds_nest_four_deep_and_no_deeper() {
    let mut project = sample();
    let wrap =
        splice_json::parse(r#"[{"cmd":"clip.compound","items":["clp_take"],"ref":"c"}]"#).unwrap();
    apply(&mut project, wrap.as_array().unwrap(), Source::Agent, false).unwrap();
    for level in 2..=5 {
        let slot = project
            .clips
            .order()
            .iter()
            .copied()
            .find(|&slot| project.clips.timing(slot).unwrap().start == 120)
            .unwrap();
        let id = project.clips.id(slot).unwrap();
        let op = splice_json::parse(&format!(r#"[{{"cmd":"clip.compound","items":["{id}"]}}]"#))
            .unwrap();
        let result = apply(&mut project, op.as_array().unwrap(), Source::Agent, false);
        if level <= 4 {
            result.unwrap();
        } else {
            assert_eq!(result.unwrap_err().error.code, "TOO_DEEP");
        }
    }
}

/// H9, D14: every path that adds or changes a transition runs one validator; refusals name
/// the rule and the fix.
#[test]
fn transitions_are_validated_on_every_path() {
    let refused = |ops: &str| {
        let mut project = sample();
        let ops = splice_json::parse(ops).unwrap();
        apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap_err()
    };
    // Generic effect paths at the wrong scope.
    for ops in [
        r#"[{"cmd":"effect.add","target":"project","type":"transition","params":{"kind":"dissolve"}}]"#,
        r#"[{"cmd":"param.set","target":"spine","path":"transition.length","value":10}]"#,
    ] {
        let error = refused(ops);
        assert!(
            matches!(error.error.code, "WRONG_KIND" | "BAD_ARGS"),
            "{ops}: {error:?}"
        );
    }
    // The first clip has no neighbor, whichever path adds it.
    let error = refused(
        r#"[{"cmd":"param.set","target":"clp_open","path":"transition.length","value":10}]"#,
    );
    assert_eq!(error.error.code, "NO_NEIGHBOR");
    // No source handles before clp_take: refused with the rule, never a silent hold.
    let error = refused(r#"[{"cmd":"transition.add","item":"clp_take","length":10}]"#);
    assert_eq!(error.error.code, "NO_HANDLES");
    let rule = error.error.rule.unwrap();
    assert_eq!(
        (rule.param.as_str(), rule.got),
        ("length", Some(splice_json::Value::Signed(10)))
    );
    assert!(error.error.fix.contains("clip.trim"));
    // Longer than the 15-frame head handle of clp_end via the generic path: refused.
    let error = refused(
        r#"[{"cmd":"transition.add","item":"clp_end","length":20},
            {"cmd":"param.set","target":"clp_end","path":"transition.length","value":40}]"#,
    );
    assert_eq!((error.op, error.error.code), (Some(1), "NO_HANDLES"));
    let error = refused(r#"[{"cmd":"transition.add","item":"clp_end","length":200}]"#);
    assert_eq!(error.error.code, "TOO_LONG");
}
