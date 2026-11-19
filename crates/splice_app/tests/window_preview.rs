#[cfg(target_os = "macos")]
#[rustfmt::skip]
#[path = "../../../pxui/tools/t/window_harness.rs"]
mod harness;

#[cfg(target_os = "macos")]
#[path = "../src/surface.rs"]
mod surface;

#[cfg(target_os = "macos")]
fn release_keepalive() {}

#[cfg(target_os = "macos")]
fn viewer_error(error: &dyn std::fmt::Display) -> splice_core::history::StoreError {
    splice_core::history::StoreError::new("VIEWER_FAILED", &error.to_string(), "Check the preview.")
}

#[cfg(target_os = "macos")]
fn window_program_4k_reuses_shared_textures_without_allocations() {
    use pxui::{FrameIn, ImageStyle, Node, Ui, UiDesc};
    use pxui_host::Window;
    use splice_av::Composition;
    use splice_model::*;
    use std::{
        path::Path,
        time::{Duration, Instant},
    };
    use surface::Surface;

    mem_sys::set_peer(pxui::library_counter);
    let size = [3840, 2160];
    let mut project = Project {
        size,
        ..Project::default()
    };
    let media = MediaId::from_serial(1);
    project.media.push(Media {
        id: media,
        kind: MediaKind::Video,
        path: "probe.mov".into(),
        bookmark: None,
        size: 0,
        mtime: 0,
        probe: Probe {
            fps: 30.0,
            frames: 30,
            width: 96,
            height: 64,
            audio: Vec::new(),
            codec: None,
        },
        take: None,
    });
    project
        .clips
        .insert(Clip {
            id: ClipId::from_serial(1),
            timing: ClipTiming {
                start: 0,
                length: 30,
                source: 0,
                speed: 1.0,
            },
            data: ClipData {
                media: Some(media),
                ..ClipData::default()
            },
        })
        .unwrap();
    let media_files = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media");
    let composition = std::rc::Rc::new(Composition::prepare(&project, &media_files).unwrap());
    let mut player = composition.player().unwrap();
    let mut window =
        Window::open_with("Splice preview test", 320, 180, false, true, false).unwrap();
    let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
    let mut surface = Surface::open(&mut window, &ui, Some(size)).unwrap();
    let mut pixels = vec![0; size[0] as usize * size[1] as usize * 4];
    let mut events = Vec::with_capacity(input::EVENT_RESERVE);
    let deadline = Instant::now() + Duration::from_secs(2);
    // Frame zero through the OGPU compositor, read back from the texture PXUI samples.
    loop {
        assert!(window.pump(&mut events));
        events.clear();
        assert!(surface.begin(&window).unwrap());
        let sample = surface.sample(&composition, &mut player).unwrap();
        let input = FrameIn::new(320.0, 180.0).time(window.time());
        ui.frame(&input, &mut |_| {}).unwrap();
        assert!(surface.draw(&window, &ui).unwrap());
        if let Some(at) = sample {
            assert_eq!(at, 0);
            assert!(surface.read_program(&mut pixels).unwrap());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "paused AVPlayer did not deliver frame zero"
        );
        window.wait(Some(window.time() + 0.01));
    }
    let left = 4 * (size[0] as usize * (size[1] as usize / 2));
    let center = left + size[0] as usize * 2;
    // The video source aspect is 3:2 and its fit into 16:9 leaves black pixels at the sides.
    assert!(pixels[left..left + 3].iter().all(|&channel| channel < 8));
    assert!(
        pixels[center..center + 3]
            .iter()
            .any(|&channel| channel > 40)
    );
    assert_eq!(pixels[center + 3], 255);

    let max_work = std::cell::Cell::new(Duration::ZERO);
    player.set_playing(true);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut last = 0;
    let mut frame = |surface: &mut Surface| loop {
        assert!(window.pump(&mut events));
        events.clear();
        assert!(surface.begin(&window).unwrap());
        let start = Instant::now();
        let sample = surface.sample(&composition, &mut player).unwrap();
        let image = surface.image();
        let input = FrameIn::new(320.0, 180.0).time(window.time());
        ui.frame(&input, &mut |ui| {
            let node = ui.node("program", Node::DEFAULT.w_px(320.0).h_px(180.0));
            ui.canvas(&node, move |paint, rect| {
                if let Some(image) = image {
                    paint.image(rect, &image, &ImageStyle::DEFAULT).unwrap();
                }
            });
        })
        .unwrap();
        assert!(surface.draw(&window, &ui).unwrap());
        if let Some(at) = sample
            && at > last
        {
            last = at;
            max_work.set(max_work.get().max(start.elapsed()));
            break;
        }
        assert!(Instant::now() < deadline, "AVPlayer stopped advancing");
        window.wait(Some(window.time() + 0.006));
    };
    for _ in 0..6 {
        frame(&mut surface);
    }
    max_work.set(Duration::ZERO);
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
        .warm(&mut || frame(&mut surface));
    for _ in 0..8 {
        frame(&mut surface);
    }
    player.set_playing(false);
    println!(
        "4K program max conversion, paint and submit {:.3} ms",
        max_work.get().as_secs_f64() * 1000.0
    );
    player.seek(11).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        assert!(window.pump(&mut events));
        events.clear();
        assert!(surface.begin(&window).unwrap());
        let sample = surface.sample(&composition, &mut player).unwrap();
        let input = FrameIn::new(320.0, 180.0).time(window.time());
        ui.frame(&input, &mut |_| {}).unwrap();
        assert!(surface.draw(&window, &ui).unwrap());
        if sample == Some(11) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "paused seek did not deliver the requested frame"
        );
        window.wait(Some(window.time() + 0.01));
    }
    assert!(surface.image().is_some());
    surface.clear_program();
    assert!(surface.image().is_none());
    surface.close().unwrap();
    window.destroy().unwrap();
}

#[cfg(target_os = "macos")]
fn main() {
    harness::run(&[(
        "window_program_4k_reuses_shared_textures_without_allocations",
        window_program_4k_reuses_shared_textures_without_allocations,
    )]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
