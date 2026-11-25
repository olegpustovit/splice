//! Read-only timeline previews. Only finished, small buffers cross to the editor thread.

use apple_sys::{audio, image, media};
use splice_model::{MediaId, MediaKind, Project};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

const THUMBNAIL_SIZE: [u32; 2] = [96, 54];
const ENVELOPE_BUCKETS: usize = 256;
const RESULT_CAPACITY: usize = 4;
const MAX_FRAME_PIXELS: u64 = 32 * 1024 * 1024;

/// A preview for one source. Either channel can be absent, including for offline media.
pub(super) struct DecodedMedia {
    /// Stable ID for the editor's preview lookup.
    pub(super) id: MediaId,
    /// Straight-alpha RGBA8 and its pixel dimensions.
    pub(super) thumbnail: Option<(Vec<u8>, [u32; 2])>,
    /// The recorded camera source, independent of the screen.
    pub(super) camera: Option<(Vec<u8>, [u32; 2])>,
    /// Linear RMS levels in source order.
    pub(super) waveform: Vec<f32>,
    /// Separate recorded microphone and system audio levels.
    pub(super) mic: Vec<f32>,
    pub(super) system: Vec<f32>,
    pub(super) events: splice_av::TimelineEvents,
}

/// Receives previews for the current editor project and suppresses wakes after drop.
pub(super) struct Decoder {
    results: Receiver<DecodedMedia>,
    live: Arc<Mutex<bool>>,
}

struct Job {
    id: MediaId,
    kind: MediaKind,
    path: String,
    size: [u32; 2],
    seconds: f64,
    fps: f64,
    screen: Option<String>,
    camera: Option<String>,
    mic: Option<String>,
    system: Option<String>,
    events: Option<String>,
}

struct Batch {
    jobs: Vec<Job>,
    package: PathBuf,
    results: SyncSender<DecodedMedia>,
    live: Arc<Mutex<bool>>,
}

struct Worker {
    running: bool,
    pending: Option<Batch>,
}

// The editor has one active project. Coalesce replacements so a slow native call cannot cause
// an accumulation of workers or previews as the user switches projects or relinks media.
static WORKER: Mutex<Worker> = Mutex::new(Worker {
    running: false,
    pending: None,
});

impl Decoder {
    /// Schedules the project's sources for the editor. The host window must outlive this decoder.
    pub(super) fn start(project: &Project, package: &Path) -> Self {
        let (output, results) = mpsc::sync_channel(RESULT_CAPACITY);
        let live = Arc::new(Mutex::new(true));
        let jobs: Vec<Job> = project
            .media
            .iter()
            .filter(|media| media.kind != MediaKind::Compound)
            .map(|media| {
                let take = media.take.as_ref();
                Job {
                    id: media.id,
                    kind: media.kind,
                    path: media.path.clone(),
                    size: [media.probe.width, media.probe.height],
                    seconds: f64::from(media.probe.frames) / media.probe.fps,
                    fps: media.probe.fps,
                    screen: take.and_then(|take| take.screen.clone()),
                    camera: take.and_then(|take| take.camera.clone()),
                    mic: take.and_then(|take| take.mic.clone()),
                    system: take.and_then(|take| take.system.clone()),
                    events: take.and_then(|take| take.events.clone()),
                }
            })
            .collect();
        if !jobs.is_empty() {
            let mut worker = WORKER.lock().unwrap();
            worker.pending = Some(Batch {
                jobs,
                package: package.to_path_buf(),
                results: output,
                live: live.clone(),
            });
            if !worker.running {
                worker.running = thread::Builder::new()
                    .name("splice-timeline-media".into())
                    .spawn(decode_batches)
                    .is_ok();
                if !worker.running {
                    worker.pending = None;
                }
            }
        }
        Self { results, live }
    }

    /// Takes a ready preview without waiting or allocating on the editor thread.
    pub(super) fn take(&self) -> Option<DecodedMedia> {
        self.results.try_recv().ok()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // Synchronize with the wake itself, without waiting for AVFoundation or ImageIO.
        *self.live.lock().unwrap() = false;
        let mut worker = WORKER.lock().unwrap();
        if worker
            .pending
            .as_ref()
            .is_some_and(|batch| Arc::ptr_eq(&batch.live, &self.live))
        {
            worker.pending = None;
        }
        // Dropping the receiver also unblocks a sender waiting for space in the result queue.
    }
}

fn decode_batches() {
    loop {
        let batch = {
            let mut worker = WORKER.lock().unwrap();
            let Some(batch) = worker.pending.take() else {
                worker.running = false;
                return;
            };
            batch
        };
        for job in batch.jobs {
            if !*batch.live.lock().unwrap() {
                break;
            }
            let preview = decode(&job, &batch.package, &batch.live);
            if preview.thumbnail.is_none()
                && preview.camera.is_none()
                && preview.waveform.is_empty()
                && preview.mic.is_empty()
                && preview.system.is_empty()
                && preview.events.clicks.is_empty()
                && preview.events.keys.is_empty()
            {
                continue;
            }
            if batch.results.send(preview).is_err() {
                break;
            }
            let live = batch.live.lock().unwrap();
            if !*live {
                break;
            }
            pxui_sdl::wake();
        }
    }
}

fn decode(job: &Job, package: &Path, live: &Mutex<bool>) -> DecodedMedia {
    let path = package.join(&job.path);
    let mut preview = DecodedMedia {
        id: job.id,
        thumbnail: None,
        camera: None,
        waveform: Vec::new(),
        mic: Vec::new(),
        system: Vec::new(),
        events: splice_av::TimelineEvents::default(),
    };
    if job.kind == MediaKind::Image {
        if pixel_count(job.size) <= MAX_FRAME_PIXELS && path.is_file() && *live.lock().unwrap() {
            preview.thumbnail = image::decode_rgba(&path)
                .ok()
                .and_then(|(pixels, size)| thumbnail(&pixels, size));
        }
        return preview;
    }

    // A take's stored path can name the directory or its primary movie.
    let folder = if job.kind == MediaKind::Take && path.is_dir() {
        path.as_path()
    } else {
        path.parent().unwrap_or(Path::new(""))
    };
    let screen = job.screen.as_ref().map(|file| folder.join(file));
    let accepting = *live.lock().unwrap();
    if job.kind == MediaKind::Take
        && let Some(file) = &job.events
        && accepting
    {
        preview.events =
            splice_av::load_timeline_events(&folder.join(file), job.fps).unwrap_or_default();
    }
    let primary = screen.as_deref().unwrap_or(&path);
    let primary_probe = probe(primary, live);
    if job.kind != MediaKind::Audio {
        if job.kind != MediaKind::Take || screen.is_some() || job.camera.is_none() {
            preview.thumbnail = primary_probe
                .as_ref()
                .and_then(|asset| video_thumbnail(primary, asset, live));
        }
        preview.camera = job.camera.as_deref().and_then(|file| {
            let camera = folder.join(file);
            video_thumbnail(&camera, &probe(&camera, live)?, live)
        });
    }
    let audio_component = |file: Option<&str>| {
        file.and_then(|file| {
            let component = folder.join(file);
            audio_waveform(&component, &probe(&component, live)?, job.seconds, live)
        })
        .unwrap_or_default()
    };
    preview.mic = audio_component(job.mic.as_deref());
    preview.system = audio_component(job.system.as_deref());
    if job.kind != MediaKind::Take {
        preview.waveform = primary_probe
            .as_ref()
            .and_then(|asset| audio_waveform(primary, asset, job.seconds, live))
            .unwrap_or_default();
    }
    preview
}

fn probe(path: &Path, live: &Mutex<bool>) -> Option<media::Probe> {
    if !*live.lock().unwrap() {
        return None;
    }
    media::probe(path).ok()
}

fn video_thumbnail(
    path: &Path,
    asset: &media::Probe,
    live: &Mutex<bool>,
) -> Option<(Vec<u8>, [u32; 2])> {
    let video = asset.video?;
    if pixel_count([video.width, video.height]) > MAX_FRAME_PIXELS || !*live.lock().unwrap() {
        return None;
    }
    // Some tracks start slightly after the asset's zero timestamp.
    let (pixels, size) = media::frame_at(path, 0.0)
        .or_else(|error| {
            if *live.lock().unwrap() && asset.duration_seconds > 0.0 {
                return media::frame_at(path, (asset.duration_seconds / 4.0).min(0.25));
            }
            Err(error)
        })
        .ok()?;
    let (mut preview, size) = thumbnail(&pixels, size)?;
    // The native movie raster is premultiplied. PXUI expects straight alpha.
    for pixel in preview.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = (u16::from(*channel) * 255 / alpha).min(255) as u8;
            }
        }
    }
    Some((preview, size))
}

fn pixel_count([width, height]: [u32; 2]) -> u64 {
    u64::from(width) * u64::from(height)
}

fn thumbnail(pixels: &[u8], [width, height]: [u32; 2]) -> Option<(Vec<u8>, [u32; 2])> {
    let count = pixel_count([width, height]);
    if count == 0 || count.checked_mul(4)? != pixels.len() as u64 {
        return None;
    }
    let [max_width, max_height] = THUMBNAIL_SIZE.map(f64::from);
    let scale = (max_width / f64::from(width))
        .min(max_height / f64::from(height))
        .min(1.0);
    let [w, h] = [width, height].map(|side| (f64::from(side) * scale).round().max(1.0) as u32);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let [width, height, w, h] = [width, height, w, h].map(|side| side as usize);
    // ponytail: sample the center of each pixel's footprint. Use a filtered resize if aliasing
    // at these small thumbnail sizes obscures source identification.
    for y in 0..h {
        let row = (2 * y + 1) * height / (2 * h);
        for x in 0..w {
            let column = (2 * x + 1) * width / (2 * w);
            let offset = (row * width + column) * 4;
            out.extend_from_slice(&pixels[offset..offset + 4]);
        }
    }
    Some((out, [w as u32, h as u32]))
}

fn audio_waveform(
    path: &Path,
    asset: &media::Probe,
    source_seconds: f64,
    live: &Mutex<bool>,
) -> Option<Vec<f32>> {
    if asset.audio.is_empty() || !*live.lock().unwrap() {
        return None;
    }
    let seconds = if source_seconds.is_finite() && source_seconds > 0.0 {
        source_seconds
    } else {
        asset.duration_seconds
    };
    audio::waveform(path, 0, seconds, ENVELOPE_BUCKETS).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_decode_real_video_and_audio_into_small_preview_buffers() {
        let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media");
        let audio_file = std::env::temp_dir().join(format!(
            "splice-timeline-waveform-{}.wav",
            std::process::id()
        ));
        let frames = 12_000_u32;
        let mut wave = b"RIFF".to_vec();
        wave.extend_from_slice(&(36 + frames * 4).to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x02\x00");
        wave.extend_from_slice(&48_000_u32.to_le_bytes());
        wave.extend_from_slice(&192_000_u32.to_le_bytes());
        wave.extend_from_slice(b"\x04\x00\x10\x00data");
        wave.extend_from_slice(&(frames * 4).to_le_bytes());
        for frame in 0..frames {
            let sample = if frame % 96 < 48 {
                16_384_i16
            } else {
                -16_384_i16
            };
            wave.extend_from_slice(&sample.to_le_bytes());
            wave.extend_from_slice(&(-sample).to_le_bytes());
        }
        std::fs::write(&audio_file, wave).unwrap();
        let ready = Mutex::new(true);
        let preview = decode(
            &Job {
                id: MediaId::from_serial(1),
                kind: MediaKind::Take,
                path: "probe.mov".into(),
                size: [0; 2],
                seconds: 0.5,
                screen: Some("probe.mov".into()),
                camera: None,
                mic: Some(audio_file.to_str().unwrap().into()),
                system: None,
                events: None,
                fps: 30.0,
            },
            &package,
            &ready,
        );
        std::fs::remove_file(audio_file).unwrap();
        let (pixels, [width, height]) = preview.thumbnail.unwrap();
        assert!(width <= THUMBNAIL_SIZE[0] && height <= THUMBNAIL_SIZE[1]);
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[..3] != [0; 3]));
        assert_eq!(preview.mic.len(), ENVELOPE_BUCKETS);
        assert!((preview.mic[40] - 0.5).abs() < 0.02);
        assert_eq!(preview.mic[220], 0.0);
    }

    #[test]
    fn timeline_thumbnail_fits_and_samples_real_pixels() {
        let mut pixels = vec![0; 192 * 108 * 4];
        let end = pixels.len();
        pixels[end - 4..].copy_from_slice(&[37, 94, 156, 255]);
        let (preview, size) = thumbnail(&pixels, [192, 108]).unwrap();
        assert_eq!(size, THUMBNAIL_SIZE);
        assert_eq!(preview.len(), 96 * 54 * 4);
        assert_eq!(&preview[preview.len() - 4..], &[37, 94, 156, 255]);
        assert_eq!(thumbnail(&pixels, [192, 107]), None);
    }
}
