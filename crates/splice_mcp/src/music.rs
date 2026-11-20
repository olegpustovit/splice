//! The music library (T45.3): a user folder of audio files the user holds rights to, indexed
//! into a catalog for `commands.describe {music}` and resolved into `music.add` ops by
//! `edit.apply`. Metadata comes from `tracks.json` sidecars and file tags (WAV INFO, ID3v2);
//! length, loudness and tempo are measured natively on macOS and cached in
//! `<library>/.splice-catalog.json`. Elsewhere only WAV lengths and the sidecars' values exist.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use super::{Json, object, text, wire::*};
use splice_core::commands::CommandError;
use splice_core::history::{Store, StoreError};
use splice_core::package::journal::Change;
use splice_model::{Frame, ItemKind, TimelineId};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const CACHE: &str = ".splice-catalog.json";
const SIDECAR: &str = "tracks.json";
const EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "caf", "m4a", "mp3", "aac", "flac",
];
/// Folders deeper than this are not indexed (and symlink loops end).
const DEPTH: usize = 8;

/// One indexed audio file.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Track {
    /// Path inside the library with `/` separators; what `music.add {track}` takes.
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    /// Seconds.
    pub length: Option<f64>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub moods: Vec<String>,
    pub tags: Vec<String>,
    /// Integrated loudness, LUFS.
    pub loudness: Option<f64>,
    /// From a sidecar; `unknown` otherwise.
    pub license: String,
}

/// Values the native backend measured for a file, cached by size and mtime.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Measured {
    length: Option<f64>,
    loudness: Option<f64>,
    bpm: Option<f64>,
}

/// The library folder: `SPLICE_MUSIC`, else `library` in `<config>/music.json`, else
/// `~/Music/Splice`.
pub(super) fn library() -> Result<PathBuf, StoreError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = config_dir(home.clone(), std::env::var_os("XDG_CONFIG_HOME"));
    let setting = config
        .as_ref()
        .and_then(|dir| fs::read_to_string(dir.join("music.json")).ok());
    resolve(std::env::var_os("SPLICE_MUSIC"), home, setting.as_deref())
}

/// Where `music.json` lives: Splice's Application Support folder on macOS, the XDG config
/// folder elsewhere.
fn config_dir(home: Option<PathBuf>, xdg: Option<OsString>) -> Option<PathBuf> {
    let home = home.filter(|home| home.is_absolute());
    if cfg!(target_os = "macos") {
        return home.map(|home| home.join("Library/Application Support/Splice"));
    }
    match xdg.map(PathBuf::from).filter(|xdg| xdg.is_absolute()) {
        Some(xdg) => Some(xdg.join("splice")),
        None => home.map(|home| home.join(".config/splice")),
    }
}

fn resolve(
    env: Option<OsString>,
    home: Option<PathBuf>,
    setting: Option<&str>,
) -> Result<PathBuf, StoreError> {
    let home = home.filter(|home| home.is_absolute());
    let expand = |raw: &str| match raw.strip_prefix("~/") {
        Some(rest) => home.as_ref().map(|home| home.join(rest)),
        None => Some(PathBuf::from(raw)).filter(|path| path.is_absolute()),
    };
    let unusable = |what: &str, raw: &str| {
        failure(
            "BAD_ARGS",
            &format!("The music library {what} {raw:?} is not an absolute path."),
            "Set SPLICE_MUSIC (or \"library\" in music.json) to an absolute folder or one under ~/.",
        )
    };
    if let Some(raw) = env.filter(|raw| !raw.is_empty()) {
        let raw = raw.to_string_lossy();
        return expand(&raw).ok_or_else(|| unusable("SPLICE_MUSIC", &raw));
    }
    if let Some(setting) = setting {
        let raw = splice_json::parse(setting)
            .ok()
            .and_then(|json| json.field("library")?.as_str().map(str::to_owned))
            .ok_or_else(|| {
                failure(
                    "BAD_ARGS",
                    "music.json has no \"library\" string.",
                    "Write {\"library\": \"/absolute/folder\"} to music.json in Splice's config folder.",
                )
            })?;
        return expand(&raw).ok_or_else(|| unusable("setting", &raw));
    }
    home.map(|home| home.join("Music/Splice")).ok_or_else(|| {
        failure(
            "NOT_FOUND",
            "No music library is configured and HOME is not set.",
            "Set SPLICE_MUSIC to the folder that holds your music.",
        )
    })
}

/// Indexes the library folder; measurements are cached next to the files.
pub(super) fn catalog(root: &Path) -> Result<Vec<Track>, StoreError> {
    if !root.is_dir() {
        return Err(failure(
            "NOT_FOUND",
            &format!("The music library {} does not exist.", root.display()),
            "Create the folder and add audio files with a tracks.json, or point SPLICE_MUSIC \
             (or \"library\" in music.json) at your music folder.",
        ));
    }
    let mut files = Vec::new();
    let mut sidecars = Vec::new();
    walk(root, root, 0, &mut files, &mut sidecars)?;
    files.sort();
    let cache = fs::read_to_string(root.join(CACHE))
        .ok()
        .and_then(|body| splice_json::parse(&body).ok());
    let mut cached = Vec::new();
    let mut changed = false;
    let mut tracks = Vec::with_capacity(files.len());
    for (path, id) in files {
        let stamp = fs::metadata(&path)
            .map(|meta| (meta.len(), mtime(&meta)))
            .unwrap_or_default();
        let hit = cache
            .as_ref()
            .and_then(|cache| cache.field("tracks")?.as_array())
            .into_iter()
            .flatten()
            .find(|entry| {
                entry.field("id").and_then(Json::as_str) == Some(id.as_str())
                    && entry.field("size").and_then(Json::as_u64) == Some(stamp.0)
                    && entry.field("mtime").and_then(Json::as_i64) == Some(stamp.1)
            })
            .map(|entry| Measured {
                length: number(entry, "length"),
                loudness: number(entry, "loudness"),
                bpm: number(entry, "bpm"),
            });
        let measured = match hit {
            Some(hit) => hit,
            None => match measure(&path) {
                Some(measured) => {
                    changed = true;
                    measured
                }
                None => Measured::default(),
            },
        };
        cached.push(object(vec![
            ("id", text(&id)),
            ("size", Json::Unsigned(stamp.0)),
            ("mtime", Json::Signed(stamp.1)),
            ("length", optional(measured.length)),
            ("loudness", optional(measured.loudness)),
            ("bpm", optional(measured.bpm)),
        ]));
        tracks.push(describe(&path, &id, &sidecars, measured));
    }
    if changed {
        let mut body = String::new();
        if splice_json::write(
            &object(vec![
                ("format", text("splice-music/1")),
                ("tracks", Json::Array(cached)),
            ]),
            &mut body,
        )
        .is_ok()
        {
            // The cache only saves re-measuring; a read-only library still indexes.
            let staged = root.join(format!("{CACHE}.tmp"));
            if fs::write(&staged, body).is_ok() {
                let _ = fs::rename(&staged, root.join(CACHE));
            }
        }
    }
    Ok(tracks)
}

fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    files: &mut Vec<(PathBuf, String)>,
    sidecars: &mut Vec<(String, Json)>,
) -> Result<(), StoreError> {
    let sidecar = dir.join(SIDECAR);
    if sidecar.is_file() {
        let body = fs::read_to_string(&sidecar).map_err(|error| unreadable(&sidecar, &error))?;
        let json = splice_json::parse(&body).map_err(|error| {
            failure(
                "BAD_MEDIA",
                &format!("{} is not valid JSON: {error}", sidecar.display()),
                "Fix tracks.json (docs/FORMAT.md, Music library) and search again.",
            )
        })?;
        sidecars.push((relative(root, dir), json));
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if depth < DEPTH {
                walk(root, &path, depth + 1, files, sidecars)?;
            }
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        {
            let id = relative(root, &path);
            files.push((path, id));
        }
    }
    Ok(())
}

fn relative(root: &Path, path: &Path) -> String {
    let parts: Vec<String> = path
        .strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.join("/")
}

/// A track's fields: its sidecar entry (nearest folder first), then file tags, then folder
/// defaults in sidecars, then the file name and `unknown` license.
fn describe(path: &Path, id: &str, sidecars: &[(String, Json)], measured: Measured) -> Track {
    // Sidecars that cover the file, nearest first, with the file's key inside each.
    let mut covering: Vec<(&Json, String)> = sidecars
        .iter()
        .filter_map(|(dir, json)| {
            let key = if dir.is_empty() {
                id.to_owned()
            } else {
                id.strip_prefix(dir.as_str())?.strip_prefix('/')?.to_owned()
            };
            Some((json, key))
        })
        .collect();
    covering.sort_by_key(|(_, key)| key.len());
    let entries: Vec<&Json> = covering
        .iter()
        .filter_map(|(json, key)| json.field("tracks")?.field(key))
        .collect();
    let defaults: Vec<&Json> = covering.iter().map(|(json, _)| *json).collect();
    let tags = read_tags(path);
    let string = |key: &str, tag: &Option<String>| {
        entries
            .iter()
            .find_map(|entry| entry.field(key)?.as_str())
            .map(str::to_owned)
            .or_else(|| tag.clone())
            .or_else(|| {
                defaults
                    .iter()
                    .find_map(|json| json.field(key)?.as_str())
                    .map(str::to_owned)
            })
            .filter(|value| !value.trim().is_empty())
    };
    let words = |keys: &[&str], tag: &[String]| -> Vec<String> {
        let from = |json: &Json| {
            keys.iter().find_map(|key| match json.field(key)? {
                Json::String(one) => Some(vec![one.to_lowercase()]),
                Json::Array(many) => Some(
                    many.iter()
                        .filter_map(Json::as_str)
                        .map(str::to_lowercase)
                        .collect(),
                ),
                _ => None,
            })
        };
        entries
            .iter()
            .find_map(|entry| from(entry))
            .or_else(|| (!tag.is_empty()).then(|| tag.to_vec()))
            .or_else(|| defaults.iter().find_map(|json| from(json)))
            .unwrap_or_default()
    };
    let declared = |key: &str| entries.iter().find_map(|entry| number(entry, key));
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().replace(['_', '-'], " "))
        .unwrap_or_default();
    Track {
        id: id.to_owned(),
        path: path.to_path_buf(),
        title: string("title", &tags.title).unwrap_or(stem),
        artist: string("artist", &tags.artist),
        // Measured values win for length and loudness; a declared tempo beats the estimate.
        length: measured.length.or(declared("length")).or(tags.length),
        bpm: declared("bpm").or(tags.bpm).or(measured.bpm),
        key: string("key", &tags.key),
        moods: words(&["mood", "moods"], &[]),
        tags: words(&["tags"], &tags.genre),
        loudness: measured.loudness.or(declared("loudness")),
        license: string("license", &None).unwrap_or_else(|| "unknown".into()),
    }
}

fn number(json: &Json, key: &str) -> Option<f64> {
    json.field(key)?.as_f64().filter(|value| value.is_finite())
}

fn optional(value: Option<f64>) -> Json {
    value.map_or(Json::Null, Json::Float)
}

#[cfg(unix)]
fn mtime(meta: &fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime()
}

#[cfg(not(unix))]
fn mtime(_: &fs::Metadata) -> i64 {
    0
}

fn unreadable(path: &Path, error: &std::io::Error) -> StoreError {
    failure(
        "NOT_FOUND",
        &format!("{} could not be read: {error}", path.display()),
        "Check the music library folder's permissions.",
    )
}

/// Length, loudness (BS.1770, the import's measure) and tempo from the native backend.
#[cfg(target_os = "macos")]
fn measure(path: &Path) -> Option<Measured> {
    const RATE: f64 = 100.0;
    let probed = splice_media::probe(Path::new("/"), path, 30).ok();
    let bpm = apple_sys::audio::envelope(path, 0, RATE)
        .ok()
        .and_then(|values| tempo(&super::analyze::onsets(&values, RATE)));
    Some(Measured {
        length: probed
            .as_ref()
            .map(|media| f64::from(media.probe.frames) / media.probe.fps),
        loudness: probed.and_then(|media| media.probe.loudness),
        bpm,
    })
}

/// Without a media backend nothing is measured (WAV headers still give a length).
#[cfg(not(target_os = "macos"))]
fn measure(_: &Path) -> Option<Measured> {
    None
}

/// Tempo from onset times: the most common beat interval, folded into 70–180 BPM. `None`
/// with fewer than eight onsets or no clear peak.
pub(super) fn tempo(onsets: &[f64]) -> Option<f64> {
    if onsets.len() < 8 {
        return None;
    }
    let mut votes = [0u32; 181];
    for pair in onsets.windows(2) {
        let gap = pair[1] - pair[0];
        if !(0.2..=2.0).contains(&gap) {
            continue;
        }
        let mut bpm = 60.0 / gap;
        while bpm < 70.0 {
            bpm *= 2.0;
        }
        while bpm > 180.0 {
            bpm /= 2.0;
        }
        votes[bpm.round() as usize] += 1;
    }
    // Neighbouring tempos vote together (jitter of one envelope sample).
    let (best, score) = (70..=180)
        .map(|bpm| {
            (
                bpm,
                votes[bpm - 1] + votes[bpm] + votes.get(bpm + 1).copied().unwrap_or(0),
            )
        })
        .max_by_key(|&(bpm, score)| (score, votes[bpm]))?;
    (score * 3 >= (onsets.len() as u32 - 1)).then_some(best as f64)
}

/// Metadata a file carries itself.
#[derive(Clone, Debug, Default, PartialEq)]
struct Tags {
    title: Option<String>,
    artist: Option<String>,
    bpm: Option<f64>,
    key: Option<String>,
    genre: Vec<String>,
    /// Seconds, from a WAV header.
    length: Option<f64>,
}

/// WAV `fmt `/`data`/`LIST INFO` chunks and ID3v2.3/2.4 text frames, from the file's head.
fn read_tags(path: &Path) -> Tags {
    let mut head = Vec::new();
    let Ok(file) = fs::File::open(path) else {
        return Tags::default();
    };
    let total = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    if file.take(1 << 20).read_to_end(&mut head).is_err() {
        return Tags::default();
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" {
        wav_tags(&head, total)
    } else if head.starts_with(b"ID3") {
        id3_tags(&head)
    } else {
        Tags::default()
    }
}

fn wav_tags(head: &[u8], total: u64) -> Tags {
    let mut tags = Tags::default();
    let mut byte_rate = 0u32;
    let mut at = 12;
    while at + 8 <= head.len() {
        let id = &head[at..at + 4];
        let size = u32::from_le_bytes([head[at + 4], head[at + 5], head[at + 6], head[at + 7]]);
        let body = at + 8;
        let end = body.saturating_add(size as usize).min(head.len());
        match id {
            b"fmt " if end >= body + 12 => {
                byte_rate = u32::from_le_bytes([
                    head[body + 8],
                    head[body + 9],
                    head[body + 10],
                    head[body + 11],
                ]);
            }
            b"data" if byte_rate > 0 => {
                // The data chunk may run past the head (and its size past a streamed file).
                let bytes = u64::from(size).min(total.saturating_sub(body as u64));
                tags.length = Some(bytes as f64 / f64::from(byte_rate));
            }
            b"LIST" if end >= body + 4 && &head[body..body + 4] == b"INFO" => {
                let mut at = body + 4;
                while at + 8 <= end {
                    let size = u32::from_le_bytes([
                        head[at + 4],
                        head[at + 5],
                        head[at + 6],
                        head[at + 7],
                    ]) as usize;
                    let value = &head[at + 8..(at + 8 + size).min(end)];
                    let value = String::from_utf8_lossy(value)
                        .trim_end_matches('\0')
                        .trim()
                        .to_owned();
                    match &head[at..at + 4] {
                        b"INAM" => tags.title = Some(value),
                        b"IART" => tags.artist = Some(value),
                        b"IGNR" => tags.genre = vec![value.to_lowercase()],
                        _ => {}
                    }
                    at += 8 + size + size % 2;
                }
            }
            _ => {}
        }
        at = body.saturating_add(size as usize + size as usize % 2);
    }
    tags
}

fn id3_tags(head: &[u8]) -> Tags {
    let mut tags = Tags::default();
    if head.len() < 10 || !matches!(head[3], 3 | 4) {
        return tags;
    }
    let synchsafe = |b: &[u8]| {
        (usize::from(b[0]) << 21)
            | (usize::from(b[1]) << 14)
            | (usize::from(b[2]) << 7)
            | usize::from(b[3])
    };
    let end = (10 + synchsafe(&head[6..10])).min(head.len());
    let mut at = 10;
    while at + 10 <= end && head[at] != 0 {
        let id = &head[at..at + 4];
        let size = if head[3] == 4 {
            synchsafe(&head[at + 4..at + 8])
        } else {
            u32::from_be_bytes([head[at + 4], head[at + 5], head[at + 6], head[at + 7]]) as usize
        };
        let body = &head[at + 10..(at + 10 + size).min(end)];
        if id[0] == b'T'
            && let Some(value) = id3_text(body)
        {
            match id {
                b"TIT2" => tags.title = Some(value),
                b"TPE1" => tags.artist = Some(value),
                b"TBPM" => tags.bpm = value.parse().ok().filter(|bpm: &f64| *bpm > 0.0),
                b"TKEY" => tags.key = Some(value),
                b"TCON" => tags.genre = vec![value.to_lowercase()],
                _ => {}
            }
        }
        at += 10 + size;
    }
    tags
}

/// An ID3v2 text frame: an encoding byte, then Latin-1, UTF-16 (with BOM), UTF-16BE or UTF-8.
fn id3_text(body: &[u8]) -> Option<String> {
    let (&encoding, bytes) = body.split_first()?;
    let utf16 = |bytes: &[u8], little: bool| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| {
                if little {
                    u16::from_le_bytes(pair)
                } else {
                    u16::from_be_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    let value = match encoding {
        0 => bytes.iter().map(|&byte| char::from(byte)).collect(),
        1 if bytes.starts_with(&[0xff, 0xfe]) => utf16(&bytes[2..], true),
        1 if bytes.starts_with(&[0xfe, 0xff]) => utf16(&bytes[2..], false),
        2 => utf16(bytes, false),
        3 => String::from_utf8_lossy(bytes).into_owned(),
        _ => return None,
    };
    // Multiple values are NUL-separated; keep the first.
    let value = value.split('\0').next().unwrap_or("").trim().to_owned();
    (!value.is_empty()).then_some(value)
}

const SEARCH_KEYS: &[&str] = &["query", "mood", "bpm", "minLength", "maxLength", "limit"];

/// `commands.describe {music:{query?, mood?, bpm?, minLength?, maxLength?, limit?}}`.
pub(super) fn search(root: &Path, args: &Json) -> Result<Json, StoreError> {
    if let Some((key, _)) = args
        .as_object()
        .ok_or_else(|| bad("music", "an object like {query, mood, bpm}"))?
        .iter()
        .find(|(key, _)| !SEARCH_KEYS.contains(&key.as_str()))
    {
        return Err(CommandError::unknown_name(
            "UNKNOWN_PARAM",
            format!("music has no field {key:?}."),
            format!("Remove {key:?}; music takes {}.", SEARCH_KEYS.join(", ")),
            key,
            &mut SEARCH_KEYS.iter().copied(),
        )
        .into());
    }
    let query = args.optional_string("query")?.unwrap_or("").to_lowercase();
    let mood = args.optional_string("mood")?.map(str::to_lowercase);
    let seconds = |key: &str| -> Result<Option<f64>, StoreError> {
        args.field(key)
            .map(|value| {
                value
                    .as_f64()
                    .filter(|value| *value >= 0.0)
                    .ok_or_else(|| bad(key, "seconds, 0 or more"))
            })
            .transpose()
    };
    let (min, max) = (seconds("minLength")?, seconds("maxLength")?);
    let bpm = args
        .field("bpm")
        .map(|value| {
            value
                .as_f64()
                .filter(|bpm| (20.0..=300.0).contains(bpm))
                .ok_or_else(|| bad("bpm", "a tempo from 20 to 300"))
        })
        .transpose()?;
    let limit = args.limit(10)?.min(50);
    let tracks = catalog(root)?;
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut ranked: Vec<(f64, &Track)> = tracks
        .iter()
        .filter_map(|track| {
            score(track, &terms, mood.as_deref(), bpm, min, max).map(|s| (s, track))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.title.cmp(&b.1.title))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    let mut moods: Vec<&str> = tracks
        .iter()
        .flat_map(|track| track.moods.iter().map(String::as_str))
        .collect();
    moods.sort_unstable();
    moods.dedup();
    let mut fields = vec![
        ("library", text(&root.to_string_lossy())),
        (
            "tracks",
            Json::Array(
                ranked
                    .iter()
                    .take(limit)
                    .map(|(_, t)| track_json(t))
                    .collect(),
            ),
        ),
        ("total", count(ranked.len())),
        ("moods", Json::Array(moods.into_iter().map(text).collect())),
    ];
    let note = if tracks.is_empty() {
        Some(format!(
            "No audio files ({}) in {}. Add tracks and a tracks.json with their license.",
            EXTENSIONS.join(", "),
            root.display()
        ))
    } else if cfg!(target_os = "macos") {
        None
    } else {
        Some(
            "Length, loudness and BPM are measured on macOS; here they come from tracks.json \
             (and WAV headers for length)."
                .into(),
        )
    };
    if let Some(note) = note {
        fields.push(("note", Json::String(note)));
    }
    Ok(object(fields))
}

/// A track's rank, or `None` when a filter excludes it. Unknown lengths and tempos pass the
/// filters but rank below known matches.
fn score(
    track: &Track,
    terms: &[&str],
    mood: Option<&str>,
    bpm: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
) -> Option<f64> {
    let mut score = 0.0;
    let has = |words: &[String], term: &str| words.iter().any(|word| word.contains(term));
    for term in terms {
        let mut hit = 0.0;
        if track.title.to_lowercase().contains(term) {
            hit += 3.0;
        }
        if track
            .artist
            .as_ref()
            .is_some_and(|artist| artist.to_lowercase().contains(term))
        {
            hit += 2.0;
        }
        if has(&track.moods, term) || has(&track.tags, term) {
            hit += 2.0;
        }
        if track
            .key
            .as_ref()
            .is_some_and(|key| key.to_lowercase().contains(term))
            || track.id.to_lowercase().contains(term)
        {
            hit += 1.0;
        }
        if hit == 0.0 {
            return None;
        }
        score += hit;
    }
    if let Some(mood) = mood {
        if !track.moods.iter().any(|m| m == mood) && !track.tags.iter().any(|t| t == mood) {
            return None;
        }
        score += 2.0;
    }
    match (bpm, track.bpm) {
        (Some(target), Some(known)) => {
            let off = (known - target).abs() / target;
            if off > 0.1 {
                return None;
            }
            score += 3.0 * (1.0 - off * 10.0);
        }
        (Some(_), None) => score -= 1.0,
        _ => {}
    }
    match track.length {
        Some(length) => {
            if min.is_some_and(|min| length < min) || max.is_some_and(|max| length > max) {
                return None;
            }
        }
        None if min.is_some() || max.is_some() => score -= 1.0,
        None => {}
    }
    Some(score)
}

fn track_json(track: &Track) -> Json {
    let round = |value: Option<f64>, scale: f64| {
        value.map_or(Json::Null, |v| Json::Float((v * scale).round() / scale))
    };
    let words = |words: &[String]| Json::Array(words.iter().map(|w| text(w)).collect());
    object(vec![
        ("track", text(&track.id)),
        ("title", text(&track.title)),
        ("artist", track.artist.as_deref().map_or(Json::Null, text)),
        ("length", round(track.length, 10.0)),
        ("bpm", round(track.bpm, 1.0)),
        ("key", track.key.as_deref().map_or(Json::Null, text)),
        ("moods", words(&track.moods)),
        ("tags", words(&track.tags)),
        ("loudness", round(track.loudness, 10.0)),
        ("license", text(&track.license)),
        ("path", text(&track.path.to_string_lossy())),
    ])
}

/// The library fields `edit.apply` adds to a `music.add {track}` op: the file, its length and
/// loudness for hosts without a media backend, and the credit (T45.3).
pub(super) fn fill(root: &Path, op: &Json) -> Result<Vec<(String, Json)>, StoreError> {
    let wanted = op
        .field("track")
        .and_then(Json::as_str)
        .ok_or_else(|| bad("track", "a track ID from commands.describe {music:{}}"))?;
    let tracks = catalog(root)?;
    let found = tracks
        .iter()
        .find(|track| track.id == wanted)
        .or_else(|| {
            let wanted = wanted.to_lowercase();
            let mut matches = tracks.iter().filter(|track| {
                track.id.to_lowercase() == wanted || track.title.to_lowercase() == wanted
            });
            // Only an unambiguous title stands in for an ID.
            match (matches.next(), matches.next()) {
                (Some(one), None) => Some(one),
                _ => None,
            }
        })
        .ok_or_else(|| not_found(root, wanted, &tracks))?;
    let mut fields = vec![
        ("track".to_owned(), text(&found.id)),
        (
            "path".to_owned(),
            Json::String(found.path.to_string_lossy().into_owned()),
        ),
        ("title".to_owned(), text(&found.title)),
        ("license".to_owned(), text(&found.license)),
    ];
    // The catalog's credit replaces whatever the op claimed.
    let mut extra = Vec::new();
    if let Some(artist) = &found.artist {
        extra.push(("artist", text(artist)));
    }
    if let Some(length) = found.length {
        extra.push(("seconds", Json::Float(length)));
    }
    if let Some(loudness) = found.loudness {
        extra.push(("loudness", Json::Float(loudness)));
    }
    fields.extend(
        extra
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    );
    Ok(fields)
}

/// `NOT_FOUND` for a track, suggesting the one whose ID or title contains what was asked.
fn not_found(root: &Path, wanted: &str, tracks: &[Track]) -> StoreError {
    let lower = wanted.to_lowercase();
    let near = tracks
        .iter()
        .find(|track| {
            track.id.to_lowercase().contains(&lower) || track.title.to_lowercase().contains(&lower)
        })
        .map(|track| track.id.clone());
    let mut message = format!(
        "No track {wanted:?} in the music library {}.",
        root.display()
    );
    if let Some(near) = &near {
        message.push_str(&format!(" Did you mean {near:?}?"));
    }
    let mut error = CommandError::rule(
        "NOT_FOUND",
        "track",
        Some(Json::String(wanted.into())),
        "a track ID from commands.describe {music:{}}",
        "Pass a track ID from commands.describe {music:{query}}.",
    );
    error.message = message;
    if let Some(rule) = &mut error.rule {
        rule.did_you_mean = near;
    }
    error.into()
}

/// The credits sidecar text for an export of `range`: every music.add track still sounding
/// in it, from the journal (so it needs no library). `None` without music.
pub(super) fn credits(store: &Store, range: [Frame; 2]) -> Option<String> {
    let project = store.project();
    // Credits by media path: an op's path, and the stored path its import resolved to.
    let mut known: Vec<(String, &Json)> = Vec::new();
    for record in store.records() {
        let Change::Edit { ops, imports, .. } = &record.batch.change else {
            continue;
        };
        for op in ops {
            if op.field("cmd").and_then(Json::as_str) != Some("music.add") {
                continue;
            }
            let Some(path) = op.field("path").and_then(Json::as_str) else {
                continue;
            };
            known.push((path.to_owned(), op));
            for import in imports.iter().filter(|import| import.request == path) {
                known.push((import.descriptor.path.clone(), op));
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for &slot in project.items.order() {
        let Some(data) = project.items.data(slot) else {
            continue;
        };
        let ItemKind::Sound(sound) = &data.kind else {
            continue;
        };
        let Some(id) = project.items.id(slot) else {
            continue;
        };
        let audible = splice_core::edit::span(project, TimelineId::Item(id))
            .is_ok_and(|span| span.start < range[1] && span.end > range[0]);
        let media = project.media.iter().find(|media| media.id == sound.media);
        let Some((_, op)) = media
            .filter(|_| audible)
            .and_then(|media| known.iter().rev().find(|(path, _)| *path == media.path))
        else {
            continue;
        };
        let field = |key: &str| op.field(key).and_then(Json::as_str).unwrap_or("");
        let title = match field("title") {
            "" => field("path"),
            title => title,
        };
        let mut line = format!("\"{title}\"");
        if !field("artist").is_empty() {
            line.push_str(&format!(" by {}", field("artist")));
        }
        let license = match field("license") {
            "" => "unknown",
            license => license,
        };
        line.push_str(&format!(" · License: {license}"));
        if !field("track").is_empty() {
            line.push_str(&format!(" · {}", field("track")));
        }
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    (!lines.is_empty()).then(|| format!("Music\n{}\n", lines.join("\n")))
}

#[cfg(test)]
#[path = "music_tests.rs"]
mod tests;
