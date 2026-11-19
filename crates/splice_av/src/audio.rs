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
