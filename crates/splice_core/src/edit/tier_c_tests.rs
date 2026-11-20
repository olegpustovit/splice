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
    // 35 rows of 0.3, then intent.steps, intent.speedTyping, caption.add (T41), item.order and
    // motion.preset (T42), html.add (T45.1), music.add (T45.3), counter.add, progress.add,
    // particles.add and look.apply (T45.4), look.save (T46.3).
    assert_eq!(rows.len(), 47);
    for command in rows {
        let described = commands::describe(Some(command.id), None).unwrap();
        let example = described.field("example").unwrap().clone();
        let mut ops = splice_json::parse(setup(command.id)).unwrap();
        let Json::Array(list) = &mut ops else {
            panic!()
        };
        list.push(example);
        let sample = with_rendered_movie(&sample, command.id);
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

/// html.add's example names a movie splice mcp rendered and the media backend registered.
fn with_rendered_movie(sample: &Project, id: &str) -> Project {
    let mut project = sample.clone();
    if id == "html.add" {
        project
            .media
            .push(rendered("media/html-5f1c.mov", MediaId::from_serial(9000)));
    }
    project
}

fn rendered(path: &str, id: MediaId) -> Media {
    let mut media = sample().media[0].clone();
    media.id = id;
    media.path = path.into();
    media.probe.frames = 90;
    media
}

#[test]
fn html_add_imports_the_rendered_movie_as_a_full_canvas_overlay() {
    let base = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"html.add","html":"<p>hi</p>","at":30,"length":90,"movie":"media/html-1.mov","ref":"page"}]"#,
    )
    .unwrap();
    let mut loads = 0;
    let mut load = |path: &str| {
        loads += 1;
        Ok(rendered(path, MediaId::from_serial(0)))
    };
    let (project, report, _) = stage_above(
        &base,
        ops.as_array().unwrap(),
        Source::Agent,
        None,
        base.serial,
        &mut load,
    )
    .unwrap();
    assert_eq!(loads, 1);
    let media = project.media.last().unwrap();
    assert_eq!(media.path, "media/html-1.mov");
    let slot = *project.items.order().last().unwrap();
    let data = project.items.data(slot).unwrap();
    let ItemKind::Media(overlay) = &data.kind else {
        panic!("{:?}", data.kind)
    };
    assert_eq!(project.items.timing(slot).unwrap().length, 90);
    assert_eq!(overlay.placement, splice_model::Placement::Full);
    assert_eq!(overlay.media, media.id);
    assert!(report.mints.iter().any(|id| id.starts_with("med_")));
    // Without a rendered movie (Linux, or a raw core call) the op says where it renders.
    let bare = splice_json::parse(r#"[{"cmd":"html.add","html":"<p>hi</p>","at":0,"length":30}]"#)
        .unwrap();
    let error = apply(
        &mut base.clone(),
        bare.as_array().unwrap(),
        Source::Agent,
        true,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("UNSUPPORTED_PLATFORM"), "{error}");
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
        // A timed mask's start/end come from from/to (timeline frames, T46.3).
        ("mask.add", "mask", &["start", "end"][..]),
        ("mask.set", "mask", &["start", "end"]),
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
fn h9_transitions_are_validated_on_every_path() {
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

/// T36.8 (M8): every scalar range a command documents (`0.5–4`, `int 1–8`, `!number 0.25–16`)
/// is the validator's range: the bounds apply, and just outside them is refused naming the
/// argument. Docs and validator cannot drift apart silently.
#[test]
fn m8_documented_ranges_match_the_validators() {
    let mut checked = 0;
    let mut drift = Vec::new();
    for row in crate::commands::COMMANDS {
        for (name, hint) in row.params {
            let spec = hint.trim_start_matches('!');
            let spec = spec
                .strip_prefix("number ")
                .or_else(|| spec.strip_prefix("int "))
                .unwrap_or(spec);
            let range = spec.split([' ', '=']).next().unwrap_or("");
            let Some((low, high)) = range.split_once('–') else {
                continue;
            };
            let (Ok(low), Ok(high)) = (low.parse::<f64>(), high.parse::<f64>()) else {
                continue;
            };
            let step = if hint.contains("int ") { 1.0 } else { 0.001 };
            let run = |value: f64| {
                let mut op = splice_json::parse(row.example).unwrap();
                let splice_json::Value::Object(fields) = &mut op else {
                    unreachable!()
                };
                let number = if step == 1.0 {
                    splice_json::Value::Signed(value as i64)
                } else {
                    splice_json::Value::Float(value)
                };
                match fields.iter_mut().find(|(key, _)| key == name) {
                    Some(field) => field.1 = number,
                    None => fields.push(((*name).into(), number)),
                }
                let mut project = sample();
                apply(&mut project, &[op], Source::Agent, false)
            };
            let names_it = |result: Result<_, crate::edit::ApplyError>| match result {
                Err(error) => error
                    .error
                    .rule
                    .is_some_and(|rule| rule.param == *name || rule.param.ends_with(name)),
                Ok(_) => false,
            };
            for inside in [low, high] {
                if names_it(run(inside)) {
                    drift.push(format!(
                        "{} {name}: documented bound {inside} is refused",
                        row.id
                    ));
                }
            }
            for outside in [low - step, high + step] {
                if !names_it(run(outside)) {
                    drift.push(format!(
                        "{} {name}: {outside} is outside the documented {low}–{high} but accepted",
                        row.id
                    ));
                }
            }
            checked += 1;
        }
    }
    assert!(drift.is_empty(), "{}", drift.join("\n"));
    assert!(checked >= 8, "{checked} ranges checked");
}

use super::tests::{created, run, start_of};

/// T37.5 M1: no edit leaves a clip shorter than three frames.
#[test]
fn m1_every_edit_keeps_clips_three_frames_long() {
    let mut project = sample();
    let error = run(
        &mut project,
        r#"[{"cmd":"item.split","item":"clp_open","at":2}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "TOO_SHORT");
    assert!(error.error.rule.is_some());
    let error = run(
        &mut project,
        r#"[{"cmd":"item.roll","item":"clp_open","by":-118}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "NO_HANDLES");
    let error = run(
        &mut project,
        r#"[{"cmd":"clip.freeze","item":"clp_open","at":119,"length":30}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "TOO_SHORT");
    // A cut that would leave a one-frame sliver takes it along.
    run(&mut project, r#"[{"cmd":"cut.range","from":1,"to":60}]"#).unwrap();
    for &slot in project.clips.order() {
        assert!(project.clips.timing(slot).unwrap().length >= 3);
    }
    assert_eq!(start_of(&project, "clp_take"), 60);
}

/// T37.5 M2: splitting a 60 fps overlay advances its source in media frames.
#[test]
fn m2_overlay_split_moves_source_in_media_frames() {
    let mut project = sample();
    let mut fast = project
        .media
        .iter()
        .find(|m| m.path.contains("broll"))
        .unwrap()
        .clone();
    fast.id = "med_fast".parse().unwrap();
    fast.path = "assets/fast.mov".into();
    fast.probe.fps = 60.0;
    fast.probe.frames = 480;
    project.media.push(fast);
    let report = run(
        &mut project,
        r#"[{"cmd":"overlay.add","kind":"media","media":"med_fast","at":200,"length":60}]"#,
    )
    .unwrap();
    let overlay = created(&report, 0);
    let report = run(
        &mut project,
        &format!(r#"[{{"cmd":"item.split","item":"{overlay}","at":230}}]"#),
    )
    .unwrap();
    let right = created(&report, 0);
    let slot = project.items.find(right.parse().unwrap()).unwrap();
    let ItemKind::Media(media) = project.items.data(slot).unwrap().kind else {
        panic!()
    };
    assert_eq!(media.source, 60, "30 timeline frames are 60 media frames");
}

/// T37.5 M3: an overlay stops at its media's end and never slides off its first frame.
#[test]
fn m3_overlays_stop_at_media_end_and_keep_sync() {
    let mut project = sample();
    let error = run(
        &mut project,
        r#"[{"cmd":"overlay.add","kind":"media","media":"med_broll","at":0,"length":300}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "OUT_OF_SOURCE");
    assert!(error.error.rule.is_some());
    // ovl_broll starts at media frame 0: extending its head would lose sync.
    let error = run(
        &mut project,
        r#"[{"cmd":"item.trim","item":"ovl_broll","edge":"in","delta":-10}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "OUT_OF_SOURCE");
    let error = run(
        &mut project,
        r#"[{"cmd":"item.trim","item":"ovl_broll","edge":"out","delta":200}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "OUT_OF_SOURCE");
}

/// T37.5 M4: splitting a joined clip keeps each seam on its content.
#[test]
fn m4_join_seams_rebase_on_split() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"},
            {"cmd":"clip.join","items":["clp_take","$cut.right"]}]"#,
    )
    .unwrap();
    let report = run(
        &mut project,
        r#"[{"cmd":"item.split","item":"clp_take","at":200}]"#,
    )
    .unwrap();
    let right = created(&report, 0);
    run(
        &mut project,
        &format!(r#"[{{"cmd":"clip.ungroup","item":"{right}"}}]"#),
    )
    .unwrap();
    let starts: Vec<Frame> = project
        .clips
        .order()
        .iter()
        .map(|&s| project.clips.timing(s).unwrap().start)
        .collect();
    assert_eq!(starts, [0, 120, 200, 300, 540]);
}

/// T37.5 M5: clip.compound takes items anchored to a clip's edge.
#[test]
fn m5_compound_keeps_edge_anchored_items() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"item.anchor","item":"ovl_broll","anchor":{"item":"clp_end","edge":"start","offset":10}},
            {"cmd":"item.anchor","item":"ovl_label","anchor":{"item":"ovl_broll","edge":"end","offset":-20}},
            {"cmd":"clip.compound","items":["clp_take","clp_end"]}]"#,
    )
    .unwrap();
}

/// T37.5 M6: an item on a clip's end edge stays there when the clip is split.
#[test]
fn m6_split_keeps_end_edge_anchors() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"item.anchor","item":"ovl_broll","anchor":{"item":"clp_take","edge":"end","offset":-80}},
            {"cmd":"item.split","item":"clp_take","at":300}]"#,
    )
    .unwrap();
    assert_eq!(start_of(&project, "ovl_broll"), 460);
}

/// T37.5 M7: keeping no shots is refused instead of deleting the timeline.
#[test]
fn m7_keep_shots_needs_a_range() {
    let mut project = sample();
    let error = run(&mut project, r#"[{"cmd":"intent.keepShots","ranges":[]}]"#).unwrap_err();
    assert_eq!(error.error.code, "BAD_ARGS");
    assert_eq!(project.clips.order().len(), 3);
}

/// T37 Low (edit side): out-of-range values are refused with the rule instead of clamped.
#[test]
fn low_out_of_range_edits_are_refused() {
    let mut project = sample();
    let refused = |project: &mut Project, ops: &str| {
        let error = run(project, ops).unwrap_err();
        assert!(error.error.rule.is_some(), "{ops}: {:?}", error.error);
        error.error.code
    };
    // clp_open is 120 frames long.
    for ops in [
        r#"[{"cmd":"item.set","items":["clp_open"],"fadeIn":200}]"#,
        r#"[{"cmd":"item.set","items":["clp_open"],"fadeIn":-1}]"#,
        r#"[{"cmd":"cut.range","from":-10,"to":20}]"#,
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":1.7,"y":0.3}]}]"#,
    ] {
        assert_eq!(refused(&mut project, ops), "BAD_ARGS", "{ops}");
    }
    assert_eq!(
        refused(&mut project, r#"[{"cmd":"marker.add","at":700}]"#),
        "PAST_END"
    );
    // Duplicating a marker onto another marker's frame is the same refusal as marker.add.
    let error = run(
        &mut project,
        r#"[{"cmd":"item.duplicate","items":["mrk_1"],"at":372}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "MARKER_EXISTS");
    // keys.set's schema names its categories, as apply takes them (A11).
    let described = commands::describe(Some("keys.set"), None).unwrap();
    let mut text = String::new();
    splice_json::write(&described, &mut text).unwrap();
    assert!(
        text.contains("all|none|shortcuts|typing|navigation|modifiers"),
        "{text}"
    );
    // SRT with an absurd hour field is malformed, not an overflow.
    assert_eq!(
        parse_cues("1\n99999999999999999:00:00,000 --> 99999999999999999:00:01,000\nhi\n"),
        None
    );
}

/// T37 Low (edit side): slide and speed keep keys on their content; lift keeps the clip's look.
#[test]
fn low_slide_speed_and_lift_keep_keys_and_looks() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"item.trim","item":"clp_end","edge":"in","delta":12},
            {"cmd":"param.keys","target":"clp_end","path":"transform.scale","keys":[{"at":0,"value":1},{"at":60,"value":2}]},
            {"cmd":"param.keys","target":"clp_open","path":"transform.scale","keys":[{"at":0,"value":1},{"at":60,"value":2}]}]"#,
    )
    .unwrap();
    let keys = |project: &Project, clip: &str| -> Vec<Frame> {
        let slot = project.clips.find(clip.parse().unwrap()).unwrap();
        let effects = &project.clips.data(slot).unwrap().appearance.effects;
        let transform = effects.iter().find(|e| e.kind == "transform").unwrap();
        match &transform
            .params
            .iter()
            .find(|p| p.name == "scale")
            .unwrap()
            .value
        {
            EffectValue::Keys(keys) => keys.iter().map(|k| k.at).collect(),
            other => panic!("{other:?}"),
        }
    };
    run(
        &mut project,
        r#"[{"cmd":"item.slide","item":"clp_take","by":-6}]"#,
    )
    .unwrap();
    assert_eq!(keys(&project, "clp_end"), [6, 66]);
    run(
        &mut project,
        r#"[{"cmd":"clip.speed","items":["clp_open"],"speed":2}]"#,
    )
    .unwrap();
    assert_eq!(keys(&project, "clp_open"), [0, 30]);
    let report = run(
        &mut project,
        r#"[{"cmd":"clip.speed","items":["clp_open"],"speed":1},{"cmd":"clip.lift","items":["clp_open"]}]"#,
    )
    .unwrap();
    let lifted = created(&report, 1);
    let slot = project.items.find(lifted.parse().unwrap()).unwrap();
    let appearance = &project.items.data(slot).unwrap().appearance;
    assert_eq!(appearance.fade_in, 15);
    assert!(appearance.effects.iter().any(|e| e.kind == "transform"));
}

/// T39.4: declared pointer marks land on the take's media in source seconds, in time order;
/// a mark outside the clip or on a non-take clip is refused with the rule.
#[test]
fn take_pointer_declares_marks_in_source_time() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"take.pointer","clip":"clp_take","marks":[{"at":300,"x":0.2,"y":0.3},{"at":180,"x":0.6,"y":0.4,"action":"click"}]}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let screen = "med_screen".parse().unwrap();
    let media = project.media.iter().find(|m| m.id == screen).unwrap();
    let marks = &media.take.as_ref().unwrap().pointer;
    // clp_take starts at 120 with source 0 at 30 fps.
    assert_eq!(marks.len(), 2);
    assert!((marks[0].t - 2.0).abs() < 1e-9 && marks[0].click);
    assert!((marks[1].t - 6.0).abs() < 1e-9 && !marks[1].click);
    for (ops, code) in [
        (
            r#"[{"cmd":"take.pointer","clip":"clp_take","marks":[{"at":600,"x":0.2,"y":0.3}]}]"#,
            "OUT_OF_RANGE",
        ),
        (
            r#"[{"cmd":"take.pointer","clip":"clp_open","marks":[{"at":10,"x":0.2,"y":0.3}]}]"#,
            "WRONG_KIND",
        ),
        (
            r#"[{"cmd":"take.pointer","clip":"clp_take","marks":[{"at":200,"x":1.2,"y":0.3}]}]"#,
            "BAD_ARGS",
        ),
    ] {
        let ops = splice_json::parse(ops).unwrap();
        let error =
            apply(&mut sample(), ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
        assert_eq!(error.error.code, code, "{ops:?}");
    }
}

/// Review #1: words on a clip with a speed effect sit where they play, inside the clip.
#[test]
fn words_follow_the_speed_effect() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"param.set","target":"clp_take","path":"speed.rate","value":1.25}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let slot = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let timing = project.clips.timing(slot).unwrap();
    let mut spans = Vec::new();
    splice_model::timing::visible_words(&project, slot, &mut |word, span| {
        spans.push((word.from, span.start, span.end));
    })
    .unwrap();
    assert!(!spans.is_empty());
    for (source, start, end) in spans {
        // clp_take: timeline 120 + source / 1.25 (source starts at 0).
        let expected = (120.0 + f64::from(source) / 1.25 + 0.5).floor() as i32;
        assert!(
            (start - expected).abs() <= 1,
            "word at source {source}: {start} vs {expected}"
        );
        assert!(
            end <= timing.start + timing.length,
            "word past the clip: {end}"
        );
    }
}

/// Review #1 follow-up: on a ramped clip, items anchored to a source frame and the words of
/// that frame resolve through one speed-aware mapping, so captions stay on their words.
#[test]
fn captions_stay_on_their_words_on_a_ramped_clip() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"param.keys","target":"clp_take","path":"speed.rate","keys":[{"at":0,"value":1},{"at":60,"value":1.5,"ease":"linear"},{"at":120,"value":1}]}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let take_id = project.clips.id(take).unwrap();
    let mut words = Vec::new();
    splice_model::timing::visible_words(&project, take, &mut |word, span| {
        words.push((word.from, span.start));
    })
    .unwrap();
    let mut checked = 0;
    for &slot in project.items.order() {
        let timing = project.items.timing(slot).unwrap();
        let splice_model::Anchor::ClipSource { clip, source } = timing.anchor else {
            continue;
        };
        if let Some(&(_, start)) = words
            .iter()
            .find(|(from, _)| clip == take_id && *from == source)
        {
            let id = project.items.id(slot).unwrap();
            assert_eq!(
                crate::edit::span(&project, TimelineId::Item(id))
                    .unwrap()
                    .start,
                start,
                "{id}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 3, "{checked} cues checked");
}

/// Review #16: re-running autoZoom replaces the clip's earlier auto zooms through delete, so
/// the result reports each one it removed.
#[test]
fn auto_zoom_rerun_reports_the_zooms_it_replaces() {
    let mut project = sample();
    let run = r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3},{"at":250,"x":0.45,"y":0.6}],"amount":1.5}]"#;
    let ops = splice_json::parse(run).unwrap();
    let first = apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let made = first.changes[0].created.len();
    assert!(made >= 2);
    let again = apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let removed = again
        .side_effects
        .iter()
        .filter(|e| matches!(e, SideEffect::Removed { .. }))
        .count();
    assert_eq!(removed, made, "{again:?}");
    let zooms = project
        .items
        .order()
        .iter()
        .filter(|&&s| matches!(project.items.data(s).unwrap().kind, ItemKind::Zoom(_)))
        .count();
    // The sample's own zoom plus one run's worth: nothing doubled.
    assert_eq!(zooms, 1 + made);
}

/// T46.2: `lead` and `tail` shape each zoom, `from`/`to` limit the clicks (and what a re-run
/// replaces) to a window, and `focus:"cursor"` makes the zooms follow the cursor.
#[test]
fn auto_zoom_takes_lead_tail_a_window_and_cursor_focus() {
    let auto = |project: &Project| -> Vec<(Frame, Frame, Zoom)> {
        project
            .items
            .order()
            .iter()
            .filter_map(|&slot| {
                let data = project.items.data(slot)?;
                let ItemKind::Zoom(zoom) = data.kind else {
                    return None;
                };
                (data.appearance.name.as_deref() == Some("Auto zoom")).then(|| {
                    let at = splice_model::timing::anchor_start(
                        project,
                        project.items.timing(slot).unwrap().anchor,
                    )
                    .unwrap();
                    (at, at + project.items.timing(slot).unwrap().length, zoom)
                })
            })
            .collect()
    };
    let mut project = sample();
    // clp_take plays source s at timeline 120 + s.
    let ops = splice_json::parse(
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3},{"at":300,"x":0.45,"y":0.6}],
             "lead":6,"tail":30,"focus":"cursor","motionBlur":0.4}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let zooms = auto(&project);
    assert_eq!(
        zooms.iter().map(|z| (z.0, z.1)).collect::<Vec<_>>(),
        [(204, 240), (414, 450)]
    );
    for (.., zoom) in &zooms {
        assert!(zoom.follow && zoom.focus.is_none() && zoom.motion_blur == 0.4);
    }
    // A window re-run: only the click inside it zooms, and only the zoom inside it is replaced.
    let ops = splice_json::parse(
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3},{"at":300,"x":0.45,"y":0.6}],
             "from":300,"to":540}]"#,
    )
    .unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let zooms = auto(&project);
    assert_eq!(zooms.len(), 2, "{zooms:?}");
    assert!(zooms[0].2.follow && zooms[0].0 == 204);
    // Default lead (0.3 s = 9 frames) and the click as a point of the crop (A5), snapped.
    assert_eq!(zooms[1].0, 420 - 9);
    assert_eq!(
        (zooms[1].2.follow, zooms[1].2.focus),
        (false, Some([0.0, 0.625]))
    );
    for ops in [
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3}],"from":300,"to":200}]"#,
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3}],"focus":"face"}]"#,
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3}],"lead":-3}]"#,
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3}],"from":900}]"#,
        r#"[{"cmd":"intent.autoZoom","item":"clp_take","clicks":[{"at":90,"x":0.7,"y":0.3}],"from":300}]"#,
    ] {
        let ops = splice_json::parse(ops).unwrap();
        let error = apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
        assert!(!error.error.fix.is_empty(), "{ops:?}: {error:?}");
    }
}

/// Recorder AC-TL-2: split, remove a piece, restore the cut: the spine plays the original
/// source frames again, at the original times. Speeds outside Recorder's 0.25–16× are refused.
#[test]
fn ac_tl_2_split_remove_and_restore_return_the_original_cut() {
    let spine = |project: &Project| -> Vec<(Option<MediaId>, Frame, Frame, Frame)> {
        let mut runs: Vec<(Option<MediaId>, Frame, Frame, Frame)> = Vec::new();
        for &slot in project.clips.order() {
            let t = project.clips.timing(slot).unwrap();
            let media = project.clips.data(slot).unwrap().media;
            // Neighbours that continue the same media at the same rate play as one run.
            match runs.last_mut() {
                Some(run)
                    if run.0 == media && run.1 + run.3 == t.start && run.2 + run.3 == t.source =>
                {
                    run.3 += t.length
                }
                _ => runs.push((media, t.start, t.source, t.length)),
            }
        }
        runs
    };
    let original = sample();
    let mut project = original.clone();
    let run = |project: &mut Project, ops: &str| {
        let ops = splice_json::parse(ops).unwrap();
        apply(project, ops.as_array().unwrap(), Source::Agent, false)
    };
    run(
        &mut project,
        r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"},
            {"cmd":"item.split","item":"$cut.right","at":360,"ref":"tail"},
            {"cmd":"item.delete","items":["$cut.right"],"ripple":true}]"#,
    )
    .unwrap();
    assert_ne!(spine(&project), spine(&original));
    run(&mut project, r#"[{"cmd":"clip.restore"}]"#).unwrap();
    assert_eq!(spine(&project), spine(&original));
    for speed in [0.25, 16.0] {
        let mut copy = original.clone();
        let ops = format!(r#"[{{"cmd":"clip.speed","items":["clp_take"],"speed":{speed}}}]"#);
        run(&mut copy, &ops).unwrap();
    }
    for speed in [0.2, 17.0] {
        let mut copy = original.clone();
        let ops = format!(r#"[{{"cmd":"clip.speed","items":["clp_take"],"speed":{speed}}}]"#);
        assert!(run(&mut copy, &ops).is_err(), "{speed}");
    }
}
