use super::*;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn pixel(rgba: &[u8], width: u32, [x, y]: [u32; 2]) -> [u8; 4] {
    let at = (y * width + x) as usize * 4;
    rgba[at..at + 4].try_into().unwrap()
}

const RED: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

#[test]
fn lottie_reports_its_size_frames_and_rate() {
    let animation = open(&fixture("square.json")).unwrap();
    assert_eq!(
        animation.info(),
        Info {
            width: 64,
            height: 64,
            frames: 30,
            fps: 30.0,
        }
    );
}

#[test]
fn lottie_square_moves_across_frames_on_a_clear_background() {
    let mut animation = open(&fixture("square.json")).unwrap();
    let mut rgba = vec![0xAB; 64 * 64 * 4];
    // Frame 0: the 16 px square is centered at (8, 32); frame 29 at (56, 32).
    animation.render(0, [64, 64], &mut rgba).unwrap();
    assert_eq!(pixel(&rgba, 64, [8, 32]), RED);
    assert_eq!(pixel(&rgba, 64, [56, 32]), CLEAR);
    assert_eq!(
        pixel(&rgba, 64, [8, 8]),
        CLEAR,
        "transparent, not left over"
    );
    animation.render(29, [64, 64], &mut rgba).unwrap();
    assert_eq!(pixel(&rgba, 64, [56, 32]), RED);
    assert_eq!(pixel(&rgba, 64, [8, 32]), CLEAR);
    // Scaled 2x: frame 0's square covers (0..32, 48..80).
    let mut big = vec![0; 128 * 128 * 4];
    animation.render(0, [128, 128], &mut big).unwrap();
    assert_eq!(pixel(&big, 128, [16, 64]), RED);
    assert_eq!(pixel(&big, 128, [40, 64]), CLEAR);
}

#[test]
fn renders_are_deterministic_whatever_came_before() {
    let render = |order: &[u32]| {
        let mut animation = open(&fixture("square.json")).unwrap();
        let mut rgba = vec![0; 64 * 64 * 4];
        for &frame in order {
            animation.render(frame, [64, 64], &mut rgba).unwrap();
        }
        rgba
    };
    let direct = render(&[15]);
    assert_eq!(render(&[15]), direct);
    assert_eq!(render(&[3, 29, 15]), direct);
    // The square is midway: centered near x = 8 + 48 * 15 / 29 ≈ 32.8.
    assert_eq!(pixel(&direct, 64, [33, 32]), RED);
    assert_eq!(pixel(&direct, 64, [8, 32]), CLEAR);
}

#[test]
fn lottie_expressions_run() {
    // Position is `[8 + time * 50, 32]`: x = 33 at frame 15 (0.5 s).
    let mut animation = open(&fixture("expression.json")).unwrap();
    let mut rgba = vec![0; 64 * 64 * 4];
    animation.render(15, [64, 64], &mut rgba).unwrap();
    assert_eq!(pixel(&rgba, 64, [33, 32]), RED);
    assert_eq!(pixel(&rgba, 64, [8, 32]), CLEAR);
    animation.render(0, [64, 64], &mut rgba).unwrap();
    assert_eq!(pixel(&rgba, 64, [8, 32]), RED);
}

#[test]
fn svg_is_a_one_frame_still_scaled_to_the_target() {
    let mut picture = open(&fixture("half.svg")).unwrap();
    assert_eq!(
        picture.info(),
        Info {
            width: 40,
            height: 20,
            frames: 1,
            fps: 0.0,
        }
    );
    let mut rgba = vec![0; 80 * 40 * 4];
    picture.render(0, [80, 40], &mut rgba).unwrap();
    assert_eq!(pixel(&rgba, 80, [10, 20]), [0, 0, 255, 255]);
    assert_eq!(pixel(&rgba, 80, [60, 20]), CLEAR);
    assert!(
        picture.render(1, [80, 40], &mut rgba).is_err(),
        "past the end"
    );
}

#[test]
fn unreadable_files_and_bad_buffers_are_errors() {
    let dir = std::env::temp_dir().join(format!("thorvg-sys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let junk = dir.join("junk.json");
    std::fs::write(&junk, b"{\"not\": \"lottie\"}").unwrap();
    assert!(open(&junk).is_err());
    assert!(open(&dir.join("missing.svg")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
    let mut animation = open(&fixture("square.json")).unwrap();
    assert!(animation.render(0, [64, 64], &mut [0; 16]).is_err());
    assert!(animation.render(0, [0, 64], &mut []).is_err());
    assert!(animation.render(30, [1, 1], &mut [0; 4]).is_err());
}

#[test]
fn warm_render_allocates_nothing() {
    let mut animation = open(&fixture("square.json")).unwrap();
    let mut rgba = vec![0; 64 * 64 * 4];
    let mut frame = 0;
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            frame = (frame + 7) % 30;
            animation.render(frame, [64, 64], &mut rgba).unwrap();
        },
    );
}
