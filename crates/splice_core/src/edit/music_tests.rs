use super::tests::{run, sample};
use super::*;
use crate::history::{Store, WriteOptions};

/// A filled `music.add` op (as `splice mcp` sends it) for a library track of `seconds`.
fn music(seconds: f64, extra: &str) -> String {
    format!(
        r#"[{{"cmd":"music.add","track":"calm/sunrise.wav","path":"/library/calm/sunrise.wav","seconds":{seconds},"loudness":-20,"title":"Sunrise","artist":"Ada","license":"CC0-1.0"{extra}}}]"#
    )
}

/// Sound items on the timeline as (start, length, volume, duck, fadeOut), in start order.
fn sounds(project: &Project) -> Vec<(Frame, Frame, f64, Option<Duck>, Frame)> {
    let mut out: Vec<_> = project
        .items
        .order()
        .iter()
        .filter_map(|&slot| {
            let data = project.items.data(slot).unwrap();
            let ItemKind::Sound(sound) = data.kind else {
                return None;
            };
            let span = time::item_span(project, slot).unwrap();
            Some((
                span.start,
                span.end - span.start,
                sound.volume,
                sound.duck,
                data.appearance.fade_out,
            ))
        })
        .collect();
    out.sort_by_key(|sound| (sound.0, sound.1));
    out
}

const DUCKED: Option<Duck> = Some(Duck {
    amount: 0.25,
    attack: 8,
    release: 15,
});

#[test]
fn music_add_fits_the_timeline_ducks_and_normalizes() {
    // The sample's spine ends at frame 660 (22 s at 30 fps); -20 LUFS raised to -14 is +6 dB.
    let gain = 10f64.powf(6.0 / 20.0);
    let mut project = sample();
    let report = run(&mut project, &music(30.0, "")).unwrap();
    assert_eq!(sounds(&project), [(0, 660, gain, DUCKED, 60)]);
    assert!(report.changes[0].summary.contains("CC0-1.0"), "{report:?}");
    let lane = project
        .lanes
        .iter()
        .find(|lane| lane.kind == LaneKind::Audio)
        .unwrap();
    assert_eq!(lane.name, "Music");
    let media = project.media.last().unwrap();
    assert_eq!(
        (media.kind, media.path.as_str(), media.probe.frames),
        (MediaKind::Audio, "/library/calm/sunrise.wav", 900)
    );

    // A short track loops to the end; only the piece the end cuts fades out.
    let mut project = sample();
    run(&mut project, &music(10.0, r#","fit":"loop","at":30"#)).unwrap();
    assert_eq!(
        sounds(&project),
        [
            (30, 300, gain, DUCKED, 0),
            (330, 300, gain, DUCKED, 0),
            (630, 30, gain, DUCKED, 30)
        ]
    );
    let mut project = sample();
    run(&mut project, &music(10.0, "")).unwrap();
    assert_eq!(sounds(&project).len(), 3, "timeline loops a short track");

    // Trim plays it once; the track ends on its own, so it keeps its ending.
    let mut project = sample();
    run(
        &mut project,
        &music(10.0, r#","fit":"trim","duck":false,"volume":0.5"#),
    )
    .unwrap();
    assert_eq!(sounds(&project), [(0, 300, 0.5, None, 0)]);
    // An explicit fade and a second track: the busy lane is skipped for a new one.
    let dusk = music(30.0, r#","fadeOut":15,"normalize":-16"#).replace("sunrise", "dusk");
    run(&mut project, &dusk).unwrap();
    let gain = 10f64.powf(4.0 / 20.0);
    assert_eq!(sounds(&project)[1], (0, 660, gain, DUCKED, 15));
    let audio = project
        .lanes
        .iter()
        .filter(|lane| lane.kind == LaneKind::Audio)
        .count();
    assert_eq!(audio, 2);
    assert_eq!(project.media.len(), 8);
    // The same file is imported once.
    run(&mut project, &music(10.0, r#","fit":"trim""#)).unwrap();
    assert_eq!(
        project
            .media
            .iter()
            .filter(|m| m.path == "/library/calm/sunrise.wav")
            .count(),
        1
    );
}

#[test]
fn music_add_refuses_unknown_licenses_and_unmeasured_levels() {
    let mut project = sample();
    for license in [r#","license":"unknown""#, r#","license":"  ""#] {
        let op = music(10.0, "").replace(r#","license":"CC0-1.0""#, license);
        let error = run(&mut project, &op).unwrap_err().error;
        assert_eq!(error.code, "UNLICENSED", "{license}");
        assert!(error.fix.contains("allowUnlicensed:true"), "{}", error.fix);
    }
    let unlicensed = music(10.0, "").replace(r#","license":"CC0-1.0""#, "");
    assert_eq!(
        run(&mut project, &unlicensed).unwrap_err().error.code,
        "UNLICENSED"
    );
    run(
        &mut project,
        &unlicensed.replace(
            r#""cmd":"music.add""#,
            r#""cmd":"music.add","allowUnlicensed":true"#,
        ),
    )
    .unwrap();

    let mut project = sample();
    let unmeasured = music(10.0, "").replace(r#","loudness":-20"#, "");
    let error = run(&mut project, &unmeasured).unwrap_err().error;
    assert_eq!(error.code, "LOUDNESS_UNKNOWN");
    run(
        &mut project,
        &unmeasured.replace(r#""cmd":"music.add""#, r#""cmd":"music.add","volume":0.4"#),
    )
    .unwrap();

    for (extra, code) in [
        (r#","at":658"#, "PAST_END"),
        (r#","fit":"stretch""#, "BAD_ARGS"),
        (r#","lane":"lan_gfx""#, "WRONG_KIND"),
    ] {
        let error = run(&mut sample(), &music(10.0, extra)).unwrap_err().error;
        assert_eq!(error.code, code, "{extra}");
    }
    // Without its library fill (a bare track), the core says how to send it.
    let error = run(
        &mut sample(),
        r#"[{"cmd":"music.add","track":"calm/sunrise.wav"}]"#,
    )
    .unwrap_err()
    .error;
    assert!(error.fix.contains("edit.apply"), "{}", error.fix);
}

/// The journal replays music.add with no library and no media backend: the import's descriptor
/// (probed on macOS, built from the catalog's length elsewhere) is recorded with the batch.
#[test]
fn music_add_replays_from_the_journal_without_the_library() {
    let dir =
        std::env::temp_dir().join(format!("splice-music-replay-{}.splice", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut store = Store::create(&dir, &sample()).unwrap();
    let session = store.start_session("test", 1).unwrap().id;
    let ops = splice_json::parse(&music(10.0, "")).unwrap();
    let options = WriteOptions {
        source: Source::Agent,
        session: Some(session),
        if_revision: Some(store.project().rev),
        ..WriteOptions::default()
    };
    // A probing loader (as on macOS) wins over the catalog's length and loudness.
    let mut probed = |path: &str| {
        assert_eq!(path, "/library/calm/sunrise.wav");
        let mut media = sample_audio(path);
        media.probe.frames = 450;
        media.probe.loudness = Some(-14.0);
        Ok(media)
    };
    store
        .apply_with_media(ops.as_array().unwrap(), &options, &mut probed)
        .unwrap();
    let options = WriteOptions {
        if_revision: Some(store.project().rev),
        ..options
    };
    // Without a backend (Linux) the catalog's length stands in.
    let other = music(4.0, "").replace("sunrise", "dusk");
    store
        .apply(
            splice_json::parse(&other).unwrap().as_array().unwrap(),
            &options,
        )
        .unwrap();
    let live = store.project().clone();
    let pieces = sounds(&live);
    assert_eq!(pieces.len(), 2 + 6, "{pieces:?}");
    assert!(
        pieces.iter().any(|p| p.1 == 450 && p.2 == 1.0),
        "the probed loudness is already -14: {pieces:?}"
    );
    drop(store);
    let reopened = Store::open(&dir).unwrap();
    assert_eq!(reopened.project().media, live.media);
    assert_eq!(sounds(reopened.project()), pieces);
    reopened.verify().unwrap();
    drop(reopened);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn sample_audio(path: &str) -> Media {
    Media {
        id: MediaId::from_serial(0),
        kind: MediaKind::Audio,
        path: path.into(),
        bookmark: None,
        size: 1,
        mtime: 1,
        probe: Probe {
            fps: 30.0,
            frames: 300,
            width: 0,
            height: 0,
            audio: vec![AudioTrack {
                name: "track 1".into(),
                channels: Some(2),
            }],
            codec: Some("pcm".into()),
            loudness: None,
        },
        take: None,
    }
}
