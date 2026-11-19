use super::{
    events::{Event, EventRing, InputEvent, InputKind},
    media,
};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/media")
        .join(name)
}

#[test]
fn cursor_images_keep_native_hotspots_and_stable_content_hashes() {
    unsafe extern "C" {
        fn splice_cursor_fixture() -> bool;
    }
    // SAFETY: the fixture creates and encodes its own bitmap cursor without accessing system input.
    assert!(unsafe { splice_cursor_fixture() });
}

#[test]
fn input_events_respect_capture_scope_and_drain_without_allocations() {
    unsafe extern "C" {
        fn splice_input_fixture(out: *mut Event) -> bool;
    }
    let mut event = Event::default();
    // SAFETY: the fixture fills a writable record and does not modify system input.
    assert!(unsafe { splice_input_fixture(&mut event) });
    let decoded = InputEvent::decode(&event).unwrap();
    assert_eq!(decoded.kind, InputKind::Key);
    assert_eq!(decoded.point, [0.25, 0.5]);
    assert_eq!(decoded.time_ns, 1_100_000_000);
    assert_eq!(decoded.text(), &[u16::from(b'a')]);
    let mut ring = EventRing::new(4, None, 0).unwrap();
    let mut scratch = [Event::default(); 1];
    let meter = splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
    meter.warm(&mut || {
        for _ in 0..1024 {
            assert!(ring.try_push(event));
            assert_eq!(ring.peek(&mut scratch), 1);
            assert_eq!(InputEvent::decode(&scratch[0]), Some(decoded));
            assert!(ring.ack(1));
        }
    });
}

#[test]
fn probes_native_fixture_movie() {
    let asset = media::probe(&fixture("probe.mov")).unwrap();
    assert!((asset.duration_seconds - 1.0).abs() < 0.002, "{asset:?}");
    let video = asset.video.expect("fixture has a video track");
    assert_eq!((video.width, video.height), (96, 64));
    assert!(
        (video.nominal_fps.unwrap() - 30.0).abs() < 0.01,
        "{video:?}"
    );
    assert_eq!(video.codec, Some(*b"avc1"));
    assert_eq!(asset.video_tracks, 1);
    assert!(asset.audio.is_empty());
}

#[test]
fn probes_stereo_audio_and_reports_native_errors() {
    let asset = media::probe(&fixture("stereo.wav")).unwrap();
    assert!(asset.video.is_none());
    assert!((asset.duration_seconds - 0.1).abs() < 0.002, "{asset:?}");
    assert_eq!(asset.audio.len(), 1);
    assert_eq!(asset.audio[0].channels, Some(2));
    assert!(
        !media::probe(&fixture("missing.mov"))
            .unwrap_err()
            .to_string()
            .is_empty()
    );
    let invalid = Path::new("probe\0.mov");
    assert!(
        media::probe(invalid)
            .unwrap_err()
            .to_string()
            .contains("NUL")
    );
}

#[test]
fn event_ring_peek_ack_wrap_overflow_and_wake() {
    static WAKE: AtomicU64 = AtomicU64::new(0);
    extern "C" fn wake(token: u64) {
        WAKE.fetch_add(token, Ordering::Relaxed);
    }
    assert!(EventRing::new(0, None, 0).is_err());
    let mut ring = EventRing::new(2, Some(wake), 3).unwrap();
    let event = |kind| Event {
        kind,
        ..Event::default()
    };
    assert!(ring.try_push(event(1)));
    assert!(ring.try_push(event(2)));
    assert!(!ring.try_push(event(3)));
    assert_eq!(ring.dropped(), 1);
    assert_eq!(WAKE.load(Ordering::Relaxed), 3);
    assert!(!ring.ack(1));

    let mut buffer = [Event::default(); 2];
    assert_eq!(ring.peek(&mut buffer), 2);
    assert_eq!(buffer, [event(1), event(2)]);
    assert!(ring.ack(1));
    assert!(ring.try_push(event(4)));
    assert!(!ring.ack(2));
    assert_eq!(ring.peek(&mut buffer), 2);
    assert_eq!(buffer, [event(2), event(4)]);
    assert!(ring.ack(2));
    assert!(ring.try_push(event(5)));
    assert_eq!(WAKE.load(Ordering::Relaxed), 6);
}

#[test]
fn event_ring_reuses_storage_after_warmup() {
    let mut ring = EventRing::new(2, None, 0).unwrap();
    let mut buffer = [Event::default(); 2];
    let meter = splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
    meter.warm(&mut || {
        assert!(ring.try_push(Event::default()));
        assert_eq!(ring.peek(&mut buffer), 1);
        assert!(ring.ack(1));
    });
}

/// T9.1: SpeechAnalyzer through the Swift shim returns timed words on device.
#[test]
fn native_speech_transcribes_synthesized_words_in_order() {
    let dir = std::env::temp_dir().join(format!("splice-speech-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (aiff, m4a) = (dir.join("speech.aiff"), dir.join("speech.m4a"));
    let said = std::process::Command::new("say")
        .args([
            "-o",
            aiff.to_str().unwrap(),
            "Today we build a frame study.",
        ])
        .status()
        .unwrap();
    assert!(said.success());
    let converted = std::process::Command::new("afconvert")
        .args([
            "-f",
            "m4af",
            "-d",
            "aac",
            aiff.to_str().unwrap(),
            m4a.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(converted.success());
    let words = crate::speech::transcribe(&m4a, "en-US").unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let text: Vec<_> = words
        .iter()
        .map(|w| {
            w.text
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect();
    assert_eq!(
        text,
        ["today", "we", "build", "a", "frame", "study"],
        "{words:?}"
    );
    assert!(
        words
            .windows(2)
            .all(|pair| pair[0].start <= pair[1].start && pair[0].end <= pair[1].end)
    );
}
