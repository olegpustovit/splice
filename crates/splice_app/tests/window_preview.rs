#[cfg(target_os = "macos")]
#[rustfmt::skip]
#[path = "../../../pxui/tools/t/window_harness.rs"]
mod harness;

#[cfg(target_os = "macos")]
mod surface {
    include!("../src/surface.rs");

    impl Surface {
        pub(super) fn set_drawable_capture(&mut self, on: bool) {
            self.frames.as_mut().unwrap().capture(on).unwrap();
        }

        pub(super) fn read_drawable_bgra(&mut self, expected: [u32; 2]) -> Vec<u8> {
            let (width, height, bgra) = self
                .frames
                .as_mut()
                .unwrap()
                .read_capture()
                .unwrap()
                .expect("capture of the submitted drawable");
            assert_eq!([width, height], expected);
            bgra
        }
    }
}

#[cfg(target_os = "macos")]
fn release_keepalive() {}

#[cfg(target_os = "macos")]
fn viewer_error(error: &dyn std::fmt::Display) -> splice_core::history::StoreError {
    splice_core::history::StoreError::new("VIEWER_FAILED", &error.to_string(), "Check the preview.")
}

#[cfg(target_os = "macos")]
fn probe_composition(size: [u32; 2]) -> std::rc::Rc<splice_av::Composition> {
    use splice_av::Composition;
    use splice_model::*;
    use std::path::Path;

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
            loudness: None,
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
    std::rc::Rc::new(Composition::prepare(&project, &media_files).unwrap())
}

#[cfg(target_os = "macos")]
fn window_program_4k_reuses_shared_textures_without_allocations() {
    use pxui::{FrameIn, ImageStyle, Node, Ui, UiDesc};
    use pxui_host::Window;
    use std::time::{Duration, Instant};
    use surface::Surface;

    mem_sys::set_peer(pxui::library_counter);
    let size = [3840, 2160];
    let composition = probe_composition(size);
    let mut player = composition.player().unwrap();
    let mut window =
        Window::open_with("Splice preview test", 320, 180, false, true, false).unwrap();
    let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
    let mut surface = Surface::open(&mut window, &ui, Some(size)).unwrap();
    let device = surface.device().clone();
    surface.set_crop_preview(Some((&[0; 16], [2, 2]))).unwrap();
    assert!(surface.crop_preview().is_some());
    surface.set_crop_preview(None).unwrap();
    assert!(surface.crop_preview().is_none());
    assert!(surface.add_thumbnail(&[0; 16], [2, 2]).is_ok());
    surface.clear_thumbnails().unwrap();
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
        let matte = surface.matte().expect("stage matte");
        let input = FrameIn::new(320.0, 180.0).time(window.time());
        ui.frame(&input, &mut |ui| {
            let node = ui.node("program", Node::DEFAULT.w_px(320.0).h_px(180.0));
            ui.canvas(&node, move |paint, rect| {
                paint
                    .image(
                        rect,
                        &matte,
                        &ImageStyle::DEFAULT.fit(pxui::draw::Fit::Fill),
                    )
                    .unwrap();
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
    // A paused raster change must draw frame 11 again without replacing or seeking
    // the player. Then the normal playback path must stay allocation-free.
    let mut step = |surface: &mut Surface, player: &mut splice_av::Player| {
        assert!(window.pump(&mut events));
        events.clear();
        assert!(surface.begin(&window).unwrap());
        let at = surface.sample(&composition, player).unwrap();
        let image = surface.image().unwrap();
        let input = FrameIn::new(320.0, 180.0).time(window.time());
        ui.frame(&input, &mut |ui| {
            let node = ui.node("program", Node::DEFAULT.w_px(320.0).h_px(180.0));
            ui.canvas(&node, move |paint, rect| {
                paint.image(rect, &image, &ImageStyle::DEFAULT).unwrap();
            });
        })
        .unwrap();
        assert!(surface.draw(&window, &ui).unwrap());
        if at.is_none() {
            window.wait(Some(window.time() + 0.006));
        }
        at
    };
    for raster in [[320, 180], [480, 270], [1920, 1080], [320, 180]] {
        surface.resize_preview(&composition, raster).unwrap();
        assert_eq!(step(&mut surface, &mut player), Some(11));
        let image = surface.image().unwrap();
        assert_eq!([image.width, image.height], raster);
        assert_eq!(surface.preview_size(), Some(raster));
    }
    let mut small = vec![0; 320 * 180 * 4];
    assert!(surface.read_program(&mut small).unwrap());
    assert!(
        small[(90 * 320 + 160) * 4..][..3]
            .iter()
            .any(|&value| value > 40)
    );

    player.set_playing(true);
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut last = 11;
    let mut frame = || loop {
        if let Some(at) = step(&mut surface, &mut player)
            && at > last
        {
            last = at;
            break;
        }
        assert!(Instant::now() < deadline, "small preview stopped advancing");
    };
    for _ in 0..4 {
        frame();
    }
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
        .warm(&mut frame);
    player.set_playing(false);
    surface.clear_program().unwrap();
    assert!(surface.image().is_none());

    let package = std::env::temp_dir().join(format!("splice-window-lab-{}", std::process::id()));
    let cache = package.join("cache");
    let frames = cache.join("frames");
    std::fs::create_dir(&package).unwrap();
    std::fs::create_dir_all(&frames).unwrap();
    let path = frames.join("frame.png");
    std::fs::write(&path, splice_av::encode_png(&[255; 16], [2, 2], 2).unwrap()).unwrap();
    let reply = [surface::ReplyFrame {
        path,
        at: Some(11),
        aspect: 1.0,
    }];
    surface
        .refresh_lab_frames(Some((1, &reply, &package)))
        .unwrap();
    let preview = surface.lab_images()[0].expect("uploaded Agent reply");
    assert!(surface.begin(&window).unwrap());
    let input = FrameIn::new(320.0, 180.0).time(window.time());
    ui.frame(&input, &mut |ui| {
        let node = ui.node(
            i64::from(reply[0].at.unwrap()),
            Node::DEFAULT.w_px(150.0 * reply[0].aspect).h_px(150.0),
        );
        ui.canvas(&node, move |paint, rect| {
            paint.image(rect, &preview, &ImageStyle::DEFAULT).unwrap();
        });
    })
    .unwrap();
    assert!(surface.draw(&window, &ui).unwrap());
    surface.refresh_lab_frames(None).unwrap();
    assert!(surface.lab_images().is_empty());
    surface
        .refresh_lab_frames(Some((1, &reply, &package)))
        .unwrap();
    assert!(surface.lab_images()[0].is_some());
    std::fs::remove_file(&reply[0].path).unwrap();
    std::fs::remove_dir(frames).unwrap();
    std::fs::remove_dir(cache).unwrap();
    std::fs::remove_dir(package).unwrap();
    surface.close().unwrap();
    assert!(ogpu_core::backend::create_queue(&device).is_err());
    assert!(ogpu_core::handle::device_destroyed(
        ogpu_core::backend::device_handle(&device),
    ));
    window.destroy().unwrap();
}

#[cfg(target_os = "macos")]
fn window_program_shuttle_crosses_clip_boundaries() {
    use pxui::{FrameIn, Ui, UiDesc};
    use pxui_host::Window;
    use splice_av::Composition;
    use splice_model::*;
    use std::{
        path::Path,
        rc::Rc,
        time::{Duration, Instant},
    };
    use surface::Surface;

    let size = [96, 64];
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
            width: size[0],
            height: size[1],
            audio: Vec::new(),
            codec: None,
            loudness: None,
        },
        take: None,
    });
    for start in [0, 30, 60] {
        project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1 + start as u64),
                timing: ClipTiming {
                    start,
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
    }
    let media_files = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media");
    let composition = Rc::new(Composition::prepare(&project, &media_files).unwrap());
    let mut player = composition.player().unwrap();
    let mut window =
        Window::open_with("Splice shuttle test", 320, 180, false, true, false).unwrap();
    let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
    let mut surface = Surface::open(&mut window, &ui, Some(size)).unwrap();
    let mut events = Vec::with_capacity(input::EVENT_RESERVE);
    let mut sample = |player: &mut splice_av::Player| {
        assert!(window.pump(&mut events));
        events.clear();
        assert!(surface.begin(&window).unwrap());
        let at = surface.sample(&composition, player);
        let input = FrameIn::new(320.0, 180.0).time(window.time());
        ui.frame(&input, &mut |_| {}).unwrap();
        assert!(surface.draw(&window, &ui).unwrap());
        window.wait(Some(window.time() + 0.008));
        at
    };
    for (rate, start) in [
        (-1, Some(35)),
        (-2, Some(35)),
        (-4, Some(35)),
        (-8, Some(35)),
        (1, Some(25)),
        (2, Some(25)),
        (4, Some(25)),
        (8, Some(25)),
        (-1, None),
        (1, None),
    ] {
        if let Some(start) = start {
            player.set_rate(0);
            player.seek(start).unwrap();
            let deadline = Instant::now() + Duration::from_millis(500);
            while sample(&mut player).unwrap() != Some(start) {
                assert!(
                    Instant::now() < deadline,
                    "shuttle start {start} did not display"
                );
            }
        }
        player.set_rate(rate);
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut last = None;
        loop {
            if let Some(at) =
                sample(&mut player).unwrap_or_else(|error| panic!("shuttle {rate}: {error}"))
            {
                last = Some(at);
                if (rate < 0 && at < 30) || (rate > 0 && at >= 30) {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "shuttle {rate} from {start:?} did not cross the edit, last frame {last:?}"
            );
        }
    }
    for (rate, end) in [(-8, 0), (8, 89)] {
        player.set_rate(rate);
        let deadline = Instant::now() + Duration::from_millis(800);
        let mut shown = None;
        while !player.ended() {
            shown = sample(&mut player).unwrap().or(shown);
            assert!(Instant::now() < deadline, "shuttle {rate} did not stop");
        }
        assert!(shown.is_some());
        player.set_rate(0);
        if shown != Some(end) {
            player.seek(end).unwrap();
            let deadline = Instant::now() + Duration::from_millis(500);
            while sample(&mut player).unwrap() != Some(end) {
                assert!(
                    Instant::now() < deadline,
                    "shuttle end {end} did not display"
                );
            }
        }
    }
    player.set_rate(0);
    surface.close().unwrap();
    window.destroy().unwrap();
}

#[cfg(target_os = "macos")]
fn window_program_drawable_survives_crop_and_resize() {
    use pxui::{FrameIn, ImageStyle, Node, Ui, UiDesc, draw::Fit};
    use pxui_host::Window;
    use splice_av::Player;
    use std::time::{Duration, Instant};
    use surface::Surface;

    const SMALL: [u32; 2] = [320, 180];
    const LARGE: [u32; 2] = [480, 270];
    let [w, h] = SMALL;
    let composition = probe_composition(SMALL);
    let mut player = composition.player().unwrap();
    let mut window = Window::open_with(
        "Splice Program drawable",
        i64::from(w),
        i64::from(h),
        false,
        true,
        false,
    )
    .unwrap();
    let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
    let mut surface = Surface::open(&mut window, &ui, Some(SMALL)).unwrap();
    let mut events = Vec::with_capacity(input::EVENT_RESERVE);
    let mut draw = |window: &mut Window, surface: &mut Surface, poll: Option<&mut Player>| {
        assert!(window.pump(&mut events));
        events.clear();
        assert!(surface.begin(window).unwrap());
        let at = poll.and_then(|player| surface.sample(&composition, player).unwrap());
        let image = surface.crop_preview().or_else(|| surface.image());
        let input = FrameIn::new(w as f32, h as f32)
            .scale(window.scale() as f32)
            .time(window.time());
        ui.frame(&input, &mut |ui| {
            let node = ui.node("program", Node::DEFAULT.w_px(w as f32).h_px(h as f32));
            if let Some(image) = image {
                ui.canvas(&node, move |paint, rect| {
                    paint
                        .image(rect, &image, &ImageStyle::DEFAULT.fit(Fit::Fill))
                        .unwrap();
                });
            }
        })
        .unwrap();
        assert!(
            surface.draw(window, &ui).unwrap(),
            "no drawable was presented"
        );
        at
    };

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(at) = draw(&mut window, &mut surface, Some(&mut player)) {
            assert_eq!(at, 0);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "paused frame zero was not delivered"
        );
        window.wait(Some(window.time() + 0.01));
    }
    let pixel = |bgra: &[u8], x: u32, y: u32| -> [u8; 4] {
        let offset = ((y * w + x) * 4) as usize;
        bgra[offset..offset + 4].try_into().unwrap()
    };
    let check_program = |bgra: &[u8]| {
        assert_eq!(pixel(bgra, w / 32, h / 2), [0, 0, 0, 255], "pillarbox");
        assert!(
            pixel(bgra, w / 2, h / 2)[..3]
                .iter()
                .any(|&value| value > 40),
            "paused picture is visible on the drawable"
        );
    };

    // Keep the original paused Program texture through the crop draw and its retirement.
    surface.set_drawable_capture(true);
    let paused_texture = surface.image().unwrap().mtl_texture;
    draw(&mut window, &mut surface, None);
    let before = surface.read_drawable_bgra(SMALL);
    check_program(&before);

    let mut crop = [0; 8 * 8 * 4];
    for (index, texel) in crop.chunks_exact_mut(4).enumerate() {
        let rgba = if index % 8 < 4 {
            [241, 37, 167, 255]
        } else {
            [31, 219, 79, 255]
        };
        texel.copy_from_slice(&rgba);
    }
    surface.set_crop_preview(Some((&crop, [8, 8]))).unwrap();
    draw(&mut window, &mut surface, None);
    let painted_crop = surface.read_drawable_bgra(SMALL);
    assert_eq!(pixel(&painted_crop, w / 4, h / 2), [167, 37, 241, 255]);
    assert_eq!(pixel(&painted_crop, 3 * w / 4, h / 2), [79, 219, 31, 255]);

    surface.set_crop_preview(None).unwrap();
    assert!(surface.crop_preview().is_none());
    assert_eq!(surface.image().unwrap().mtl_texture, paused_texture);
    draw(&mut window, &mut surface, None);
    let after_crop = surface.read_drawable_bgra(SMALL);
    assert!(
        before == after_crop,
        "closing Crop changed the paused Program drawable"
    );

    for raster in [LARGE, SMALL] {
        surface.resize_preview(&composition, raster).unwrap();
        assert!(surface.image().is_none());
        assert_eq!(draw(&mut window, &mut surface, Some(&mut player)), Some(0));
        let image = surface.image().unwrap();
        assert_eq!([image.width, image.height], raster);
        let presented = surface.read_drawable_bgra(SMALL);
        check_program(&presented);
        if raster == SMALL {
            assert!(
                before == presented,
                "raster round trip changed the paused Program drawable"
            );
        }
    }

    surface.set_drawable_capture(false);
    surface.close().unwrap();
    window.destroy().unwrap();
}

#[cfg(target_os = "macos")]
fn main() {
    harness::run(&[
        (
            "window_program_drawable_survives_crop_and_resize",
            window_program_drawable_survives_crop_and_resize,
        ),
        (
            "window_program_4k_reuses_shared_textures_without_allocations",
            window_program_4k_reuses_shared_textures_without_allocations,
        ),
        (
            "window_program_shuttle_crosses_clip_boundaries",
            window_program_shuttle_crosses_clip_boundaries,
        ),
    ]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
