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
    // Canonical JSON has no trailing newline (splice_json round-trips every fixture byte for byte).
    splice_json::write(&value, &mut bytes).unwrap();
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
    assert_eq!(error.error.code, "BAD_ARGS");
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
            r#"{"cmd":"zoom.set","zoom":"ovl_detail","amount":9}"#,
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

/// D12: zoom segments apply, validate with the rule, round-trip, and a 0.3 `motion` item
/// (ovl_detail in the sample, 1 → 1.16) loads as a segment that holds 1.16.
#[test]
fn zoom_segments_apply_validate_round_trip_and_migrate_motion_items() {
    let mut project = sample();
    let slot = project.items.find("ovl_detail".parse().unwrap()).unwrap();
    let ItemKind::Zoom(migrated) = project.items.data(slot).unwrap().kind else {
        panic!("motion migrates to zoom")
    };
    assert_eq!((migrated.amount, migrated.focus), (1.16, None));
    run(
        &mut project,
        r#"[{"cmd":"zoom.set","zoom":"ovl_detail","focus":{"x":0.25,"y":0.75},"spring":"smooth"},
            {"cmd":"zoom.add","at":0,"length":30,"amount":2,"focus":{"x":0.5,"y":0.1},"mode":"content"}]"#,
    )
    .unwrap();
    let ItemKind::Zoom(zoom) = project.items.data(slot).unwrap().kind else {
        panic!("zoom")
    };
    assert_eq!(
        (zoom.focus, zoom.spring),
        (Some([0.25, 0.75]), Spring::Smooth)
    );
    for op in [
        r#"{"cmd":"zoom.set","zoom":"ovl_detail","focus":{"x":2,"y":0}}"#,
        r#"{"cmd":"zoom.add","at":0,"focus":{"x":0.5}}"#,
        r#"{"cmd":"zoom.add","at":0,"amount":0.8}"#,
        r#"{"cmd":"zoom.add","at":0,"spring":"bouncy"}"#,
        r#"{"cmd":"zoom.set","zoom":"ovl_hook","amount":2}"#,
    ] {
        let error = run(&mut project, &format!("[{op}]")).unwrap_err();
        assert!(
            matches!(error.error.code, "BAD_ARGS" | "WRONG_KIND"),
            "{op}: {error:?}"
        );
    }
    // Saved and reopened, the segment survives; null focus keeps the current framing.
    let dir = std::env::temp_dir().join(format!("splice-zoom-{}.splice", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    drop(crate::package::Package::create(&dir, &project).unwrap());
    let (package, reopened) = crate::package::Package::open(&dir).unwrap();
    drop(package);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        reopened.items.data(slot).unwrap().kind,
        project.items.data(slot).unwrap().kind
    );
    run(
        &mut project,
        r#"[{"cmd":"zoom.set","zoom":"ovl_detail","focus":null}]"#,
    )
    .unwrap();
    let ItemKind::Zoom(zoom) = project.items.data(slot).unwrap().kind else {
        panic!("zoom")
    };
    assert_eq!(zoom.focus, None);
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

/// A 10-second stereo music file measured at -20 LUFS.
pub(super) fn with_music(project: &mut Project) {
    project.media.push(Media {
        id: "med_music".parse().unwrap(),
        kind: MediaKind::Audio,
        path: "music.m4a".into(),
        bookmark: None,
        size: 1,
        mtime: 0,
        probe: Probe {
            fps: 30.0,
            frames: 300,
            width: 0,
            height: 0,
            audio: vec![AudioTrack {
                name: "audio".into(),
                channels: Some(2),
            }],
            codec: Some("aac".into()),
            loudness: Some(-20.0),
        },
        take: None,
    });
}

#[test]
fn tier_b_music_lands_on_an_audio_lane_with_volume_duck_and_loudness() {
    let mut project = sample();
    with_music(&mut project);
    let report = run(
        &mut project,
        r#"[{"cmd":"clip.insert","media":"med_music","ref":"$music"},
            {"cmd":"audio.set","items":["$music"],"normalize":-14,"duck":{"amount":0.3}}]"#,
    )
    .unwrap();
    let commands::Target::Timeline(TimelineId::Item(id)) = &report.changes[0].created[0] else {
        panic!("created item")
    };
    let slot = project.items.find(*id).unwrap();
    let data = project.items.data(slot).unwrap();
    let lane = project
        .lanes
        .iter()
        .find(|lane| lane.id == data.lane)
        .unwrap();
    assert_eq!(lane.kind, LaneKind::Audio);
    assert_eq!(
        project.items.timing(slot).unwrap(),
        ItemTiming {
            anchor: Anchor::Pinned(0),
            length: 300
        }
    );
    let ItemKind::Sound(sound) = data.kind else {
        panic!("sound")
    };
    // -20 LUFS raised to -14: +6 dB.
    assert!(
        (sound.volume - 10f64.powf(6.0 / 20.0)).abs() < 1e-9,
        "{}",
        sound.volume
    );
    assert_eq!(
        sound.duck,
        Some(Duck {
            amount: 0.3,
            attack: 8,
            release: 15
        })
    );
    // The spine is untouched, and the sound survives a save.
    assert_eq!(project.clips.order().len(), 3);
    let dir = std::env::temp_dir().join(format!("splice-music-{}.splice", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    drop(crate::package::Package::create(&dir, &project).unwrap());
    let (package, reopened) = crate::package::Package::open(&dir).unwrap();
    drop(package);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        reopened.items.data(slot).unwrap().kind,
        project.items.data(slot).unwrap().kind
    );
    assert_eq!(reopened.media.last().unwrap().probe.loudness, Some(-20.0));
    for (op, code) in [
        (
            r#"{"cmd":"audio.set","items":["ovl_hook"],"volume":1}"#,
            "WRONG_KIND",
        ),
        (r#"{"cmd":"audio.set","items":["ovl_hook"]}"#, "BAD_ARGS"),
        (
            r#"{"cmd":"audio.set","clip":"clp_take","component":"mic"}"#,
            "BAD_ARGS",
        ),
        (
            r#"{"cmd":"clip.insert","media":"med_music","lane":"lan_gfx"}"#,
            "WRONG_KIND",
        ),
    ] {
        let error = run(&mut project, &format!("[{op}]")).unwrap_err();
        assert_eq!(error.error.code, code, "{op}");
    }
}

#[test]
fn tier_b_transcript_set_replaces_words_and_feeds_captions() {
    let mut project = sample();
    let old = project.transcripts[0].hash;
    let report = run(
        &mut project,
        r#"[{"cmd":"transcript.set","media":"med_screen","words":[{"text":"Hello","from":6,"to":17},{"text":"world","from":20,"to":30}]},
            {"cmd":"captions.generate","maxWords":2}]"#,
    )
    .unwrap();
    let transcript = &project.transcripts[0];
    assert_ne!(transcript.hash, old);
    let texts: Vec<_> = transcript
        .words
        .iter()
        .map(|w| (w.id.to_string(), w.text.as_str()))
        .collect();
    assert_eq!(
        texts,
        [("w1".to_owned(), "Hello"), ("w2".to_owned(), "world")]
    );
    // Captions on the old words settle away; the new ones caption the new words.
    let cues: Vec<_> = project
        .items
        .order()
        .iter()
        .filter_map(|&slot| match &project.items.data(slot).unwrap().kind {
            ItemKind::Caption { words, .. } => Some(words.len()),
            _ => None,
        })
        .collect();
    assert_eq!(cues, [2]);
    assert!(
        report
            .side_effects
            .iter()
            .any(|e| matches!(e, SideEffect::Removed { .. }))
    );
    for op in [
        r#"{"cmd":"transcript.set","media":"med_screen","words":[{"text":"a","from":5,"to":4}]}"#,
        r#"{"cmd":"transcript.set","media":"med_screen","words":[{"text":"late","from":10,"to":9999}]}"#,
        r#"{"cmd":"transcript.set","media":"clp_take","words":[]}"#,
    ] {
        assert!(run(&mut project, &format!("[{op}]")).is_err(), "{op}");
    }
}

/// Eval friction (tighten-talk): after cut.words in the same batch, captions.generate {clips:[take]}
/// must caption every piece of the take, and a full-length pinned watermark must follow the ripple.
#[test]
fn tier_b_cut_words_then_captions_cover_the_whole_take_and_watermark_follows() {
    let mut project = sample();
    let logo = project.items.find("ovl_logo".parse().unwrap()).unwrap();
    let end = |p: &Project| {
        p.clips
            .order()
            .iter()
            .map(|&s| {
                let t = p.clips.timing(s).unwrap();
                t.start + t.length
            })
            .max()
            .unwrap()
    };
    assert_eq!(
        (project.items.timing(logo).unwrap().length, end(&project)),
        (660, 660)
    );
    run(
        &mut project,
        r#"[{"cmd":"cut.words","words":["w7"]},{"cmd":"captions.generate","clips":["clp_take"],"maxWords":3}]"#,
    )
    .unwrap();
    let spine_end = end(&project);
    assert!(spine_end < 660, "the cut rippled the spine");
    assert_eq!(
        project.items.timing(logo).unwrap().length,
        spine_end,
        "the watermark follows the ripple"
    );
    // Every live piece of the take carries captions.
    let take = project
        .media
        .iter()
        .find(|m| m.path.contains("screen"))
        .map(|m| m.id);
    let pieces: Vec<ClipId> = project
        .clips
        .order()
        .iter()
        .filter(|&&s| project.clips.data(s).unwrap().media == take)
        .map(|&s| project.clips.id(s).unwrap())
        .collect();
    assert!(pieces.len() >= 2, "{pieces:?}");
    for piece in pieces {
        let captioned = project.items.order().iter().any(|&s| {
            matches!(project.items.data(s).unwrap().kind, ItemKind::Caption { .. })
                && matches!(project.items.timing(s).unwrap().anchor, Anchor::ClipSource { clip, .. } if clip == piece)
        });
        assert!(captioned, "{piece} has captions");
    }
}

/// T36.4: keys on a parameter the evaluator does not animate are refused with the rule, never
/// accepted and rendered as the default.
#[test]
fn keys_on_a_non_keyable_parameter_are_refused() {
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"param.keys","target":"clp_take","path":"window.padding","keys":[{"at":0,"value":0.05},{"at":30,"value":0.2}]}]"#,
    )
    .unwrap();
    let error = apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
    assert_eq!(error.error.code, "NOT_KEYABLE");
    assert_eq!(error.error.rule.unwrap().param, "padding");
    assert!(error.error.fix.contains("none"), "{}", error.error.fix);
}
