use super::tests::{sample, start_of};
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
            {"cmd":"audio.set","clip":"clp_take","component":"mic","volume":1.5},
            {"cmd":"keys.hide","clip":"clp_take","events":[4,2,4],"hidden":true},
            {"cmd":"clip.crop","items":["clp_take"],"rect":null,"fit":"fill"}]"#,
    )
    .unwrap();
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let data = project.clips.data(take).unwrap();
    assert_eq!(data.enabled.camera, Some(false));
    assert_eq!(data.volume.mic, 1.5);
    assert_eq!(data.hidden_keys, [2, 4]);
    assert_eq!(
        data.framing,
        Framing {
            crop: None,
            fit: Fit::Fill
        }
    );
    for (op, code) in [
        // Out of range is refused, never clamped (Low list).
        (
            r#"{"cmd":"audio.set","clip":"clp_take","component":"mic","volume":3}"#,
            "BAD_ARGS",
        ),
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
            ItemKind::Caption { words, edits, .. } => (words.clone(), edits.len()),
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
        [
            ("w:screen:1".to_owned(), "Hello"),
            ("w:screen:2".to_owned(), "world")
        ]
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

/// Media of a caption's anchor clip, and the media its words name.
fn caption_media(project: &Project, slot: Slot) -> (Option<MediaId>, Vec<Option<MediaId>>) {
    let ItemKind::Caption { words, .. } = &project.items.data(slot).unwrap().kind else {
        panic!("not a caption")
    };
    let Anchor::ClipSource { clip, .. } = project.items.timing(slot).unwrap().anchor else {
        panic!("caption anchored to a clip")
    };
    let clip = project
        .clips
        .data(project.clips.find(clip).unwrap())
        .unwrap();
    (clip.media, words.iter().map(|w| w.media()).collect())
}

fn captions(project: &Project) -> Vec<Slot> {
    project
        .items
        .order()
        .iter()
        .copied()
        .filter(|&s| {
            matches!(
                project.items.data(s).unwrap().kind,
                ItemKind::Caption { .. }
            )
        })
        .collect()
}

fn media_length(project: &Project, media: &str) -> Frame {
    let media: MediaId = media.parse().unwrap();
    project
        .clips
        .order()
        .iter()
        .filter(|&&s| project.clips.data(s).unwrap().media == Some(media))
        .map(|&s| project.clips.timing(s).unwrap().length)
        .sum()
}

/// T37.1 C1: two takes that both say "alpha bravo" keep their words apart.
#[test]
fn c1_word_ids_are_project_unique() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"clip.insert","media":"med_webcam"},
            {"cmd":"transcript.set","media":"med_screen","words":[{"text":"alpha","from":6,"to":17},{"text":"bravo","from":20,"to":30}]},
            {"cmd":"transcript.set","media":"med_webcam","words":[{"text":"alpha","from":6,"to":17},{"text":"bravo","from":20,"to":30}]},
            {"cmd":"captions.generate","maxWords":8}]"#,
    )
    .unwrap();
    // One cue per take, never "alpha bravo alpha bravo"; each cue's words are its clip's.
    let cues = captions(&project);
    assert_eq!(cues.len(), 2);
    for &cue in &cues {
        let (media, words) = caption_media(&project, cue);
        assert_eq!(words, [media, media]);
    }
    let before = (
        media_length(&project, "med_screen"),
        media_length(&project, "med_webcam"),
    );
    // A legacy ID two transcripts share is refused live, with the rule.
    let error = run(&mut project, r#"[{"cmd":"cut.words","words":["w1"]}]"#).unwrap_err();
    assert_eq!(error.error.code, "AMBIGUOUS_WORD");
    assert!(error.error.rule.is_some());
    // The webcam's "alpha" is cut from the webcam take, not the screen take.
    run(
        &mut project,
        r#"[{"cmd":"cut.words","words":["w:webcam:1"]}]"#,
    )
    .unwrap();
    assert_eq!(media_length(&project, "med_screen"), before.0);
    assert!(media_length(&project, "med_webcam") < before.1);
    // Regenerating the webcam's captions leaves the screen take's cue alone.
    let webcam: MediaId = "med_webcam".parse().unwrap();
    let cam = project
        .clips
        .order()
        .iter()
        .rfind(|&&s| project.clips.data(s).unwrap().media == Some(webcam))
        .map(|&s| project.clips.id(s).unwrap())
        .unwrap();
    run(
        &mut project,
        &format!(r#"[{{"cmd":"captions.generate","clips":["{cam}"]}}]"#),
    )
    .unwrap();
    let screen: MediaId = "med_screen".parse().unwrap();
    assert!(
        captions(&project)
            .iter()
            .any(|&s| caption_media(&project, s).0 == Some(screen))
    );
}

/// T37.1 C1: a pre-0.4 package's `w<n>` IDs bind to their transcript's media on load.
#[test]
fn c1_legacy_word_ids_migrate_on_load() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
    let path = std::env::temp_dir().join(format!("splice-c1-{}.splice", std::process::id()));
    std::fs::create_dir_all(path.join("transcripts")).unwrap();
    for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
        let text = std::fs::read_to_string(source.join(file)).unwrap();
        std::fs::write(path.join(file), text.replace("\"w:screen:", "\"w")).unwrap();
    }
    let (package, legacy) = crate::package::Package::open(&path).unwrap();
    drop(package);
    std::fs::remove_dir_all(path).unwrap();
    let current = sample();
    assert_eq!(legacy.transcripts, current.transcripts);
    for (&a, &b) in legacy.items.order().iter().zip(current.items.order()) {
        assert_eq!(legacy.items.data(a), current.items.data(b));
        assert_eq!(legacy.items.timing(a), current.items.timing(b));
    }
}

fn cause_of(report: &ApplyReport, item: &str) -> Option<Cause> {
    let item: ItemId = item.parse().unwrap();
    report
        .side_effects
        .iter()
        .rev()
        .find_map(|effect| match *effect {
            SideEffect::Removed { id, cause, .. }
            | SideEffect::Reanchored { id, cause, .. }
            | SideEffect::Bumped { id, cause, .. }
                if id == item =>
            {
                Some(cause)
            }
            _ => None,
        })
}

/// T37.2 H2b: a cut that removes an item's whole parent clip deletes the item.
#[test]
fn h2b_cutting_a_whole_parent_cascades() {
    let mut project = sample();
    let report = run(&mut project, r#"[{"cmd":"cut.range","from":0,"to":120}]"#).unwrap();
    assert!(project.items.find("ovl_hook".parse().unwrap()).is_none());
    assert_eq!(cause_of(&report, "ovl_hook"), Some(Cause::Cascade));
}

/// T37.3 D21: a partial cut re-parents (default), deletes or trims the items on the cut piece.
#[test]
fn d21_partial_cut_reparents_deletes_or_trims_connected_items() {
    let base = sample();
    let broll = || "ovl_broll".parse::<ItemId>().unwrap();
    // ovl_broll starts at 420 on clp_take for 75 frames; cut 410–440 out from under it.
    let mut project = base.clone();
    let report = run(&mut project, r#"[{"cmd":"cut.range","from":410,"to":440}]"#).unwrap();
    assert_eq!(cause_of(&report, "ovl_broll"), Some(Cause::Reparent));
    let at = span(&project, TimelineId::Item(broll())).unwrap();
    assert_eq!((at.start, at.end), (410, 485));

    let mut project = base.clone();
    let report = run(
        &mut project,
        r#"[{"cmd":"cut.range","from":410,"to":440,"connected":"delete"}]"#,
    )
    .unwrap();
    assert!(project.items.find(broll()).is_none());
    assert_eq!(cause_of(&report, "ovl_broll"), Some(Cause::Cut));

    let mut project = base.clone();
    run(
        &mut project,
        r#"[{"cmd":"cut.range","from":410,"to":440,"connected":"trim"}]"#,
    )
    .unwrap();
    let slot = project.items.find(broll()).unwrap();
    let at = span(&project, TimelineId::Item(broll())).unwrap();
    assert_eq!((at.start, at.end), (410, 465));
    let ItemKind::Media(overlay) = project.items.data(slot).unwrap().kind else {
        panic!()
    };
    assert_eq!(
        overlay.source, 20,
        "the trimmed overlay keeps showing the same frames"
    );

    // item.delete of a clip cascades unless told to re-parent.
    let mut project = base.clone();
    let report = run(
        &mut project,
        r#"[{"cmd":"item.delete","items":["clp_open"],"connected":"reparent"}]"#,
    )
    .unwrap();
    assert_eq!(cause_of(&report, "ovl_hook"), Some(Cause::Reparent));
    assert_eq!(start_of(&project, "ovl_hook"), 0);
}

/// T37.6 M19: a ripple cut shortens the item that runs into it, so the next item on its lane
/// closes up instead of bumping; a real bump names the item in the way.
#[test]
fn m19_ripple_shortens_spanning_items_and_bumps_carry_a_cause() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"lane.create","kind":"graphics","name":"Notes","ref":"$lane"},
            {"cmd":"title.add","at":300,"length":60,"lane":"$lane","text":"A"},
            {"cmd":"title.add","at":365,"length":30,"lane":"$lane","text":"B"}]"#,
    )
    .unwrap();
    let a = super::tests::created(&report, 1);
    let b = super::tests::created(&report, 2);
    let report = run(&mut project, r#"[{"cmd":"cut.range","from":320,"to":340}]"#).unwrap();
    assert!(
        !report
            .side_effects
            .iter()
            .any(|e| matches!(e, SideEffect::Bumped { .. })),
        "{:?}",
        report.side_effects
    );
    let a_span = span(&project, TimelineId::Item(a.parse().unwrap())).unwrap();
    assert_eq!((a_span.start, a_span.end), (300, 340));
    assert_eq!(start_of(&project, &b), 345);
    let report = run(
        &mut project,
        &format!(r#"[{{"cmd":"item.move","items":["{b}"],"to":310}}]"#),
    )
    .unwrap();
    assert_eq!(
        cause_of(&report, &b),
        Some(Cause::Overlap(a.parse().unwrap()))
    );
}

/// T37.4 H3: an item anchored to a word survives cutting the word and replacing the transcript.
#[test]
fn h3_word_anchored_items_survive_cuts_and_new_transcripts() {
    let base = sample();
    let anchor = r#"{"cmd":"item.anchor","item":"ovl_label","anchor":{"word":"w:screen:12"}}"#;
    let mut project = base.clone();
    run(&mut project, &format!("[{anchor}]")).unwrap();
    let slot = project.items.find("ovl_label".parse().unwrap()).unwrap();
    assert!(matches!(
        project.items.timing(slot).unwrap().anchor,
        Anchor::ClipSource { .. }
    ));
    run(
        &mut project,
        r#"[{"cmd":"cut.words","words":["w:screen:12"]}]"#,
    )
    .unwrap();
    run(
        &mut project,
        r#"[{"cmd":"transcript.set","media":"med_screen","words":[{"text":"new","from":6,"to":17}]}]"#,
    )
    .unwrap();
    let mut project = base;
    run(
        &mut project,
        &format!(r#"[{anchor},{{"cmd":"item.delete","items":["clp_take"]}}]"#),
    )
    .unwrap();
}

/// T37.4 H4: a freeze near the media's end is speed 0, so later edits of it work.
#[test]
fn h4_freeze_near_media_end_stays_editable() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"clip.freeze","item":"clp_take","at":535,"length":150}]"#,
    )
    .unwrap();
    let freeze = super::tests::created(&report, 0);
    run(
        &mut project,
        &format!(r#"[{{"cmd":"item.trim","item":"{freeze}","edge":"out","delta":30}}]"#),
    )
    .unwrap();
    let report = run(
        &mut project,
        &format!(r#"[{{"cmd":"item.split","item":"{freeze}","at":600}}]"#),
    )
    .unwrap();
    let right = super::tests::created(&report, 0);
    let held = |project: &Project, id: &str| {
        let slot = project.clips.find(id.parse().unwrap()).unwrap();
        project.clips.timing(slot).unwrap().source
    };
    assert_eq!(held(&project, &freeze), held(&project, &right));
    run(
        &mut project,
        &format!(r#"[{{"cmd":"clip.speed","items":["{right}"],"speed":0.5}}]"#),
    )
    .unwrap();
}

/// T37.4 H5: a duplicated take's captions stay on the copy.
#[test]
fn h5_duplicate_keeps_captions_on_the_copy() {
    let mut project = sample();
    let report = run(
        &mut project,
        r#"[{"cmd":"item.duplicate","items":["clp_take"]}]"#,
    )
    .unwrap();
    let copy: ClipId = super::tests::created(&report, 0).parse().unwrap();
    let take: ClipId = "clp_take".parse().unwrap();
    let on = |clip: ClipId| {
        captions(&project)
            .iter()
            .filter(|&&s| {
                matches!(project.items.timing(s).unwrap().anchor,
                    Anchor::ClipSource { clip: c, .. } if c == clip)
            })
            .count()
    };
    assert_eq!(on(take), 10);
    assert_eq!(on(copy), 10);
}

/// T37.6 M17: a brand color recolors windows but never turns one on.
#[test]
fn m17_style_color_does_not_enable_windows() {
    let mut project = sample();
    project.effects.retain(|e| e.kind != "window");
    run(&mut project, r##"[{"cmd":"style.set","color":"#1e3a8a"}]"##).unwrap();
    assert!(!project.effects.iter().any(|e| e.kind == "window"));
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let window = project
        .clips
        .data(take)
        .unwrap()
        .appearance
        .effects
        .iter()
        .find(|e| e.kind == "window")
        .unwrap();
    assert!(
        window
            .params
            .iter()
            .any(|p| p.name == "color" && p.value == EffectValue::Text("#1e3a8a".into()))
    );
}

/// T37.6 M18: a take's camera look needs a camera.
#[test]
fn m18_effect_add_checks_take_components() {
    let mut project = sample();
    run(
        &mut project,
        r#"[{"cmd":"clip.insert","media":"med_webcam"}]"#,
    )
    .unwrap();
    let webcam = project.clips.order().last().copied().unwrap();
    let id = project.clips.id(webcam).unwrap();
    // med_webcam records a camera and a mic, no screen events: no cursor, no keystrokes.
    run(
        &mut project,
        &format!(r#"[{{"cmd":"effect.add","target":"{id}","type":"camera"}}]"#),
    )
    .unwrap();
    let error = run(
        &mut project,
        &format!(r#"[{{"cmd":"effect.add","target":"{id}","type":"cursor"}}]"#),
    )
    .unwrap_err();
    assert_eq!(error.error.code, "WRONG_KIND");
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    project.clips.data_mut(take).unwrap().enabled.camera = None;
    let error = run(
        &mut project,
        r#"[{"cmd":"effect.add","target":"clp_take","type":"camera"}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "WRONG_KIND");
}

/// T37.6 M21: lane, spine and project transforms apply as parents (T42.4, drawn by the
/// evaluator: splice_eval motion_tests), so they are accepted like an item's own.
#[test]
fn m21_lane_and_project_transforms_apply() {
    let mut project = sample();
    for target in ["lan_gfx", "lan_spine", "project", "ovl_label"] {
        run(
            &mut project,
            &format!(r#"[{{"cmd":"effect.add","target":"{target}","type":"transform","params":{{"x":0.1}}}}]"#),
        )
        .unwrap();
    }
}

/// Review #15: zoom.remove deletes a camera segment and refuses anything else.
#[test]
fn zoom_remove_deletes_only_zooms() {
    let mut project = sample();
    let ops = splice_json::parse(r#"[{"cmd":"zoom.remove","zoom":"ovl_detail"}]"#).unwrap();
    apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    assert!(project.items.find("ovl_detail".parse().unwrap()).is_none());
    let ops = splice_json::parse(r#"[{"cmd":"zoom.remove","zoom":"ovl_hook"}]"#).unwrap();
    let error = apply(&mut sample(), ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
    assert_eq!(error.error.code, "WRONG_KIND");
}

/// M9: a zoom made by overlay.add carries the validated default amount (it takes no amount,
/// so nothing unchecked can reach the camera).
#[test]
fn m9_overlay_add_zoom_is_a_validated_default() {
    let mut project = sample();
    let ops = splice_json::parse(r#"[{"cmd":"overlay.add","kind":"zoom","at":200}]"#).unwrap();
    let report = apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let Some(crate::commands::Target::Timeline(TimelineId::Item(id))) =
        report.changes[0].created.first().cloned()
    else {
        panic!("zoom created");
    };
    let slot = project.items.find(id).unwrap();
    let ItemKind::Zoom(zoom) = project.items.data(slot).unwrap().kind else {
        panic!("a zoom")
    };
    assert!((Zoom::AMOUNT[0]..=Zoom::AMOUNT[1]).contains(&zoom.amount));
    let ops =
        splice_json::parse(r#"[{"cmd":"overlay.add","kind":"zoom","at":200,"amount":9}]"#).unwrap();
    let error = apply(&mut sample(), ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
    assert_eq!(error.error.code, "UNKNOWN_PARAM");
}
