use super::{
    events::{Event, EventRing},
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
