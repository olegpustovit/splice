//! Audio effects and pan (PLAN.md T19.1–T19.2) are rendered offline into the package cache and
//! played from there; the composition only mixes gains.
use crate::Error;
use splice_eval::{Audio, Evaluator};
use std::path::{Path, PathBuf};

/// The file and track a lowered audio input plays: the source itself, or its processed copy in
/// `<package>/cache/audio/`, keyed by a hash of the input and the chain.
pub(crate) fn processed(
    package: &Path,
    evaluator: &Evaluator,
    audio: Audio,
    path: &Path,
    index: u32,
) -> Result<(PathBuf, u32), Error> {
    let fx = evaluator.audio_fx(audio.item).unwrap_or_default();
    // ponytail: pan is constant per item (its value at the item's first audible frame); keyed
    // pan needs a per-segment render if a mix asks for moving pans.
    if fx == splice_eval::AudioFx::default() && audio.pan == 0.0 {
        return Ok((path.to_path_buf(), index));
    }
    let chain = apple_sys::audio::Chain {
        eq: fx.eq,
        compressor: fx.compressor,
        gate: fx.gate,
        isolation: fx.isolation,
        pan: audio.pan,
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &byte in bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
    };
    feed(path.as_os_str().as_encoded_bytes());
    feed(&index.to_le_bytes());
    feed(format!("{chain:?}").as_bytes());
    if let Ok(meta) = std::fs::metadata(path) {
        feed(&meta.len().to_le_bytes());
    }
    let dir = package.join("cache").join("audio");
    let out = dir.join(format!("{hash:016x}.caf"));
    if !out.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| Error::Input(e.to_string()))?;
        let partial = dir.join(format!("{hash:016x}.partial.caf"));
        apple_sys::audio::render(path, index, &partial, &chain)?;
        std::fs::rename(&partial, &out).map_err(|e| Error::Input(e.to_string()))?;
    }
    Ok((out, 0))
}

/// Seconds of the generated mouse click (`clickSound`, T46.2).
pub(crate) const CLICK_SECONDS: f64 = 0.06;
/// Sample rate of the generated click.
const CLICK_RATE: u32 = 48_000;

/// The click mixed in at each recorded press: a short, bright tick (a decaying 3.2 kHz partial
/// over seeded noise), generated and written once to `<package>/cache/audio/` as 16-bit mono
/// WAV. No asset ships with the app.
pub(crate) fn click(package: &Path) -> Result<PathBuf, Error> {
    let dir = package.join("cache").join("audio");
    let out = dir.join("click-v1.wav");
    if out.exists() {
        return Ok(out);
    }
    let failed = |e: std::io::Error| Error::Input(format!("{}: {e}", dir.display()));
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let partial = dir.join("click-v1.partial.wav");
    std::fs::write(&partial, click_wav()).map_err(failed)?;
    std::fs::rename(&partial, &out).map_err(failed)?;
    Ok(out)
}

/// The click as WAV bytes (deterministic).
pub(crate) fn click_wav() -> Vec<u8> {
    let count = (CLICK_SECONDS * f64::from(CLICK_RATE)) as u32;
    let mut seed = 0x2545_f491_u32;
    let data = 2 * count;
    let mut out = Vec::with_capacity(44 + data as usize);
    for chunk in [
        &b"RIFF"[..],
        &(36 + data).to_le_bytes(),
        b"WAVEfmt ",
        &16u32.to_le_bytes(),
        &1u16.to_le_bytes(),
        &1u16.to_le_bytes(),
        &CLICK_RATE.to_le_bytes(),
        &(2 * CLICK_RATE).to_le_bytes(),
        &2u16.to_le_bytes(),
        &16u16.to_le_bytes(),
        b"data",
        &data.to_le_bytes(),
    ] {
        out.extend_from_slice(chunk);
    }
    for i in 0..count {
        let t = f64::from(i) / f64::from(CLICK_RATE);
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = f64::from(seed) / f64::from(u32::MAX) * 2.0 - 1.0;
        let tone = (std::f64::consts::TAU * 3200.0 * t).sin();
        // A 0.3 ms attack, then a fast decay: a tick, not a beep.
        let envelope = (t / 0.0003).min(1.0) * (-t / 0.006).exp();
        let sample = (0.55 * tone + 0.45 * noise) * envelope * 0.7;
        out.extend_from_slice(&((sample * f64::from(i16::MAX)) as i16).to_le_bytes());
    }
    out
}
