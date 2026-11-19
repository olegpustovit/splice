//! `media.analyze` (PLAN.md T20.1, D6): loudness and pauses everywhere; peaks, beats, sound
//! labels, shots, on-screen text, faces, people and the person matte from native analysis on
//! macOS. Raw findings are cached per media in `<package>/analysis/<media>/`; each answer maps
//! them onto the clip's timeline frames.
// The native half (native.rs) is macOS-only; Linux builds keep the portable helpers for tests.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use super::{Json, object, text, wire::*};
use splice_core::history::StoreError;
use splice_model::*;
use std::path::{Path, PathBuf};

pub(super) const KINDS: &[&str] = &[
    "loudness", "pauses", "peaks", "beats", "sounds", "shots", "text", "faces", "people", "matte",
];

/// A clip's source window on its media clock.
#[derive(Clone, Copy)]
pub(super) struct Window {
    pub start: Frame,
    pub length: Frame,
    pub source: f64,
    pub rate: f64,
    pub fps: f64,
}

impl Window {
    /// Timeline frame of a media time in seconds, if the clip shows it.
    pub fn frame(&self, seconds: f64) -> Option<Frame> {
        let offset = (seconds * self.fps - self.source) / self.rate;
        (offset >= 0.0 && offset < f64::from(self.length))
            .then(|| self.start + offset.round() as Frame)
    }

    /// Media seconds the clip covers.
    pub fn seconds(&self) -> (f64, f64) {
        let from = self.source / self.fps;
        (from, from + f64::from(self.length) * self.rate / self.fps)
    }
}

/// Cut points where feature-print distance jumps: above `floor` and 3σ over the mean.
pub(super) fn shots(prints: &[(f64, f64)], floor: f64) -> Vec<f64> {
    let values: Vec<f64> = prints.iter().skip(1).map(|&(_, d)| d).collect();
    if values.is_empty() {
        return Vec::new();
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let sd = (values.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt();
    let threshold = floor.max(mean + 3.0 * sd);
    prints
        .iter()
        .skip(1)
        .filter(|&&(_, d)| d > threshold)
        .map(|&(t, _)| t)
        .collect()
}

/// Onsets in an RMS envelope sampled at `rate`: rises over an adaptive threshold, at least
/// 0.25 s apart (beats for music, hits for SFX).
pub(super) fn onsets(envelope: &[f32], rate: f64) -> Vec<f64> {
    let flux: Vec<f64> = envelope
        .windows(2)
        .map(|w| f64::from((w[1] - w[0]).max(0.0)))
        .collect();
    let window = (rate * 0.5).max(1.0) as usize;
    let gap = (rate * 0.25).max(1.0) as usize;
    let mut out = Vec::new();
    let mut last = None;
    for i in 1..flux.len().saturating_sub(1) {
        let lo = i.saturating_sub(window);
        let hi = (i + window).min(flux.len());
        let local = flux[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
        let peak = flux[i] >= flux[i - 1] && flux[i] > flux[i + 1];
        if peak && flux[i] > 1.5 * local + 1e-3 && last.is_none_or(|l: usize| i - l >= gap) {
            out.push((i + 1) as f64 / rate);
            last = Some(i);
        }
    }
    out
}

/// Consecutive windows with the same label merged: (from, to, label).
pub(super) fn runs(labels: &[(f64, f64, String)]) -> Vec<(f64, f64, String)> {
    let mut out: Vec<(f64, f64, String)> = Vec::new();
    for (start, duration, label) in labels {
        match out.last_mut() {
            Some(last) if last.2 == *label && *start <= last.1 + 0.01 => last.1 = start + duration,
            _ => out.push((*start, start + duration, label.clone())),
        }
    }
    out
}

pub(super) fn analyze(
    projects: &mut super::project::Projects,
    args: &Json,
) -> Result<Json, StoreError> {
    let store = projects.lookup(args)?;
    let p = store.project();
    let id: ClipId = args
        .string("clip")?
        .parse()
        .map_err(|_| bad("clip", "a clip ID from timeline.snapshot"))?;
    let slot = p.clips.find(id).ok_or_else(|| {
        failure(
            "NOT_FOUND",
            &format!("No clip {id}."),
            "Call timeline.snapshot for clip IDs.",
        )
    })?;
    let kinds: Vec<String> = match args.field("kinds") {
        None => vec!["loudness".into(), "pauses".into()],
        Some(value) => value
            .as_array()
            .ok_or_else(|| bad("kinds", "an array of analysis kinds"))?
            .iter()
            .map(|kind| {
                kind.as_str()
                    .filter(|k| KINDS.contains(k))
                    .map(str::to_owned)
                    .ok_or_else(|| bad("kinds", &KINDS.join(", ")))
            })
            .collect::<Result<_, _>>()?,
    };
    let mut fields = vec![("revision", rev(p.rev)), ("clip", display(&id))];
    if kinds.iter().any(|k| k == "loudness" || k == "pauses") {
        let summary = super::media::audio(store, args)?;
        if kinds.iter().any(|k| k == "loudness") {
            fields.push((
                "components",
                summary.field("components").cloned().unwrap_or(Json::Null),
            ));
        }
        if kinds.iter().any(|k| k == "pauses") {
            fields.push((
                "pauses",
                summary.field("silences").cloned().unwrap_or(Json::Null),
            ));
        }
    }
    let native: Vec<&str> = kinds
        .iter()
        .map(String::as_str)
        .filter(|k| !matches!(*k, "loudness" | "pauses"))
        .collect();
    if !native.is_empty() {
        let media = p
            .clips
            .data(slot)
            .and_then(|d| d.media)
            .and_then(|m| p.media.iter().find(|x| x.id == m))
            .ok_or_else(|| {
                failure(
                    "NO_MEDIA",
                    "A gap has nothing to analyze.",
                    "Pass a clip with media.",
                )
            })?;
        let window = Window {
            start: p.clips.timing(slot).expect("live clip").start,
            length: p.clips.timing(slot).expect("live clip").length,
            source: f64::from(p.clips.timing(slot).expect("live clip").source),
            rate: timing::source_rate(p, slot).unwrap_or(1.0),
            fps: media.probe.fps,
        };
        // Long media analyze on a worker (T15.3); the job's result is this same answer.
        #[cfg(target_os = "macos")]
        if f64::from(media.probe.frames) / media.probe.fps > super::background::LONG_SECONDS
            || args.flag("async")?
        {
            let package = store.path().to_path_buf();
            let media = media.clone();
            let kinds: Vec<String> = native.iter().map(|k| (*k).to_owned()).collect();
            let project = args.string("projectId")?.to_owned();
            let work = move || {
                let kinds: Vec<&str> = kinds.iter().map(String::as_str).collect();
                let mut fields = fields;
                fields.extend(super::platform::analyze(&package, &media, &window, &kinds)?);
                Ok(object(fields))
            };
            return Ok(super::background::Background::spawn(
                projects,
                "analyze",
                &project,
                Box::new(work),
                None,
            ));
        }
        fields.extend(super::platform::analyze(
            store.path(),
            media,
            &window,
            &native,
        )?);
    }
    Ok(object(fields))
}

/// The file behind a media's video or audio component (takes keep components in a folder).
pub(super) fn component(package: &Path, media: &Media, audio: bool) -> PathBuf {
    let base = package.join(&media.path);
    let name = media.take.as_ref().and_then(|take| {
        if audio {
            take.mic.as_deref().or(take.system.as_deref())
        } else {
            take.screen.as_deref()
        }
    });
    match name {
        Some(file) if base.is_dir() => base.join(file),
        Some(file) => base.with_file_name(file),
        None => base,
    }
}

/// `<package>/analysis/<media>/`.
pub(super) fn cache_dir(package: &Path, media: MediaId) -> PathBuf {
    package.join("analysis").join(media.to_string())
}

/// A cached analysis file, valid while the source file's size and mtime match.
pub(super) fn cached(dir: &Path, name: &str, source: &Path) -> Option<String> {
    let stamp = stamp(source)?;
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    let (head, body) = text.split_once('\n')?;
    (head == stamp).then(|| body.to_owned())
}

pub(super) fn store_cache(dir: &Path, name: &str, source: &Path, body: &str) {
    if let Some(stamp) = stamp(source)
        && std::fs::create_dir_all(dir).is_ok()
    {
        let _ = std::fs::write(dir.join(name), format!("{stamp}\n{body}"));
    }
}

fn stamp(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("v1 {} {mtime}", meta.len()))
}

pub(super) fn rect_json(rect: [f64; 4]) -> Json {
    object(vec![
        ("x", Json::Float((rect[0] * 1e4).round() / 1e4)),
        ("y", Json::Float((rect[1] * 1e4).round() / 1e4)),
        ("w", Json::Float((rect[2] * 1e4).round() / 1e4)),
        ("h", Json::Float((rect[3] * 1e4).round() / 1e4)),
    ])
}

pub(super) fn label(value: &str) -> Json {
    text(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_cuts_beats_and_label_runs_from_raw_findings() {
        let prints: Vec<(f64, f64)> = (0..40)
            .map(|i| {
                (
                    f64::from(i) * 0.5,
                    if i == 12 || i == 30 {
                        0.9
                    } else {
                        0.05 + 0.01 * f64::from(i % 3)
                    },
                )
            })
            .collect();
        assert_eq!(shots(&prints, 0.3), [6.0, 15.0]);
        // A click every 0.5 s at 100 Hz.
        let envelope: Vec<f32> = (0..300)
            .map(|i| if i % 50 == 10 { 1.0 } else { 0.05 })
            .collect();
        let beats = onsets(&envelope, 100.0);
        assert_eq!(beats, [0.1, 0.6, 1.1, 1.6, 2.1, 2.6]);
        let merged = runs(&[
            (0.0, 1.0, "speech".into()),
            (0.5, 1.0, "speech".into()),
            (1.5, 1.0, "music".into()),
        ]);
        assert_eq!(
            merged,
            [(0.0, 1.5, "speech".into()), (1.5, 2.5, "music".into())]
        );
        let window = Window {
            start: 100,
            length: 60,
            source: 30.0,
            rate: 2.0,
            fps: 60.0,
        };
        assert_eq!(window.frame(1.0), Some(115));
        assert_eq!(window.frame(0.1), None);
        assert_eq!(window.seconds(), (0.5, 2.5));
    }
}
