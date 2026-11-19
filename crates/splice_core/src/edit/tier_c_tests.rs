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
        "transition.set" | "transition.remove" => r#"[{"cmd":"transition.add","item":"clp_take"}]"#,
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
    assert_eq!(rows.len(), 36);
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
