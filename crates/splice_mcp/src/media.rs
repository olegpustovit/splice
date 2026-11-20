use super::{
    Json, object,
    project::{lane_kind, media_path},
    text, timeline,
    wire::*,
};
use splice_core::{
    edit::span,
    history::{Store, StoreError},
};
use splice_model::{timing, *};
use std::{fs, path::Path};

struct ClipClock {
    timing: ClipTiming,
    rate: f64,
}

impl ClipClock {
    fn new(p: &Project, slot: Slot) -> Result<Self, StoreError> {
        Ok(Self {
            timing: p.clips.timing(slot).expect("live clip"),
            rate: timing::source_rate(p, slot).map_err(|error| {
                failure(
                    "INVARIANT",
                    &format!("{error:?}"),
                    "Check the clip source frame rate.",
                )
            })?,
        })
    }

    fn source_end(&self) -> f64 {
        f64::from(self.timing.source) + f64::from(self.timing.length) * self.rate
    }

    fn has(&self, source: f64) -> bool {
        source >= f64::from(self.timing.source) && source < self.source_end()
    }

    fn at(&self, source: f64) -> Frame {
        let offset = ((source - f64::from(self.timing.source)) / self.rate).round();
        self.timing.start + (offset as Frame).clamp(0, self.timing.length)
    }

    fn start(&self, source: f64) -> Frame {
        self.at(source)
            .min(self.timing.end().expect("valid clip") - 1)
    }

    fn interval(&self, from: f64, to: f64) -> (Frame, Frame) {
        let start = self.start(from);
        (start, self.at(to).max(start + 1))
    }
}

fn clip(p: &Project, args: &Json) -> Result<(ClipId, Slot), StoreError> {
    let id: ClipId = args
        .string("clip")?
        .parse()
        .map_err(|_| bad("clip", "a clip ID from timeline.snapshot"))?;
    let slot = p.clips.find(id).ok_or_else(|| {
        failure(
            "NOT_FOUND",
            &format!("No clip {id}."),
            "Call timeline.snapshot to find a current clip ID.",
        )
    })?;
    Ok((id, slot))
}

fn clip_media(p: &Project, slot: Slot) -> Option<&Media> {
    let id = p.clips.data(slot)?.media?;
    p.media.iter().find(|media| media.id == id)
}

fn clip_transcript(p: &Project, slot: Slot) -> Option<&Transcript> {
    let id = p.clips.data(slot)?.media?;
    p.transcripts
        .iter()
        .find(|transcript| transcript.media == id)
}

pub(super) fn transcript(p: &Project, args: &Json) -> Result<Json, StoreError> {
    if let Some(format) = args.optional_string("captions")? {
        return captions(p, args, format, None);
    }
    if args.field("item").is_some() {
        return item_transcript(p, args);
    }
    let (id, slot) = if args.field("clip").is_some() {
        clip(p, args)?
    } else {
        let slot = p
            .clips
            .order()
            .iter()
            .copied()
            .find(|&slot| clip_transcript(p, slot).is_some())
            .ok_or_else(no_transcript)?;
        (p.clips.id(slot).unwrap(), slot)
    };
    let transcript = clip_transcript(p, slot).ok_or_else(no_transcript)?;
    let clock = ClipClock::new(p, slot)?;
    let total = transcript.words.len();
    let start = args.cursor('w', total)?;
    let end = start.saturating_add(args.limit(60)?).min(total);
    let words = transcript.words[start..end]
        .iter()
        .map(|word| {
            let from = f64::from(word.from);
            let span = clock
                .has(from)
                .then(|| clock.interval(from, f64::from(word.to).min(clock.source_end())));
            let mut fields = vec![
                ("id", display(&word.id)),
                ("text", text(&word.text)),
                ("sourceFrom", frame(word.from)),
                ("sourceTo", frame(word.to)),
                ("frame", span.map_or(Json::Null, |(from, _)| frame(from))),
                ("to", span.map_or(Json::Null, |(_, to)| frame(to))),
                ("length", frame(word.to - word.from)),
            ];
            if let Some(confidence) = word.confidence {
                fields.push(("confidence", Json::Float(confidence)));
            }
            if filler(&word.text) {
                fields.push(("filler", Json::Bool(true)));
            }
            object(fields)
        })
        .collect();
    let fillers = transcript
        .words
        .iter()
        .filter(|word| filler(&word.text))
        .map(|word| display(&word.id))
        .collect();
    // Abandoned attempts a later phrase restarts (T19.3): their word IDs, ready for cut.words.
    let spoken: Vec<(&str, Frame, Frame)> = transcript
        .words
        .iter()
        .map(|w| (w.text.as_str(), w.from, w.to))
        .collect();
    let gap = (media_fps(p, slot) * 0.4).round() as Frame;
    let retakes = splice_core::edit::retakes(&spoken, gap)
        .into_iter()
        .map(|(from, to)| {
            Json::Array(
                transcript.words[from..to]
                    .iter()
                    .map(|w| display(&w.id))
                    .collect(),
            )
        })
        .collect();
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("hash", hash(transcript.hash)),
        ("clip", display(&id)),
        ("language", text(&transcript.language)),
        ("words", Json::Array(words)),
        ("fillers", Json::Array(fillers)),
        ("retakes", Json::Array(retakes)),
        ("nextCursor", next_cursor('w', end, total)),
    ]))
}

use splice_core::edit::is_filler as filler;

/// The media behind a sound item or media overlay (voice-overs, podcast tracks, music).
pub(super) fn item_media(
    p: &Project,
    args: &Json,
) -> Result<(ItemId, Slot, MediaId, Frame), StoreError> {
    let id: ItemId = args
        .string("item")?
        .parse()
        .map_err(|_| bad("item", "a sound or media overlay ID (ovl_…)"))?;
    let slot = p.items.find(id).ok_or_else(|| {
        failure(
            "NOT_FOUND",
            &format!("No item {id}."),
            "Call timeline.snapshot to find a current item ID.",
        )
    })?;
    match p.items.data(slot).expect("live item").kind {
        ItemKind::Sound(sound) => Ok((id, slot, sound.media, sound.source)),
        ItemKind::Media(overlay) => Ok((id, slot, overlay.media, overlay.source)),
        _ => Err(bad("item", "a sound or media overlay (it has audio)")),
    }
}

/// `transcript.get {item}`: an item's words, on the timeline through the item's start.
fn item_transcript(p: &Project, args: &Json) -> Result<Json, StoreError> {
    let (id, slot, media, source) = item_media(p, args)?;
    let transcript = p
        .transcripts
        .iter()
        .find(|t| t.media == media)
        .ok_or_else(|| {
            failure(
                "NOT_FOUND",
                &format!("{id} has no transcript yet."),
                "Call transcript.get {item, transcribe:true, sessionId} to transcribe it.",
            )
        })?;
    let span = span(p, TimelineId::Item(id))
        .map_err(|error| failure(error.code, &error.message, &error.fix))?;
    let rate = p
        .media
        .iter()
        .find(|m| m.id == media)
        .map_or(1.0, |m| m.probe.fps / f64::from(p.fps));
    let length = p.items.timing(slot).expect("live item").length;
    let total = transcript.words.len();
    let start = args.cursor('w', total)?;
    let end = start.saturating_add(args.limit(60)?).min(total);
    let on_timeline = |source_frame: Frame| {
        let at = (f64::from(source_frame - source) / rate).round() as Frame;
        (0..=length).contains(&at).then_some(span.start + at)
    };
    let words = transcript.words[start..end]
        .iter()
        .map(|word| {
            let mut fields = vec![
                ("id", display(&word.id)),
                ("text", text(&word.text)),
                ("sourceFrom", frame(word.from)),
                ("sourceTo", frame(word.to)),
                ("frame", on_timeline(word.from).map_or(Json::Null, frame)),
                ("to", on_timeline(word.to).map_or(Json::Null, frame)),
                ("length", frame(word.to - word.from)),
            ];
            if filler(&word.text) {
                fields.push(("filler", Json::Bool(true)));
            }
            object(fields)
        })
        .collect();
    let fillers = transcript
        .words
        .iter()
        .filter(|word| filler(&word.text))
        .map(|word| display(&word.id))
        .collect();
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("hash", hash(transcript.hash)),
        ("item", display(&id)),
        ("language", text(&transcript.language)),
        ("words", Json::Array(words)),
        ("fillers", Json::Array(fillers)),
        ("nextCursor", next_cursor('w', end, total)),
    ]))
}

fn media_fps(p: &Project, slot: Slot) -> f64 {
    clip_media(p, slot).map_or(f64::from(p.fps), |m| m.probe.fps)
}

fn no_transcript() -> StoreError {
    failure(
        "NOT_FOUND",
        "No transcript is available for this clip.",
        "Call project.info for transcript state or pass a clip with a transcript.",
    )
}

/// Caption cues as JSON, SRT or WebVTT. With `window` (an export's `[from, to)`), only cues
/// inside it, clipped to it and timed from its start (M11).
pub(super) fn captions(
    p: &Project,
    args: &Json,
    format: &str,
    window: Option<[Frame; 2]>,
) -> Result<Json, StoreError> {
    if !matches!(format, "json" | "srt" | "vtt") {
        return Err(bad("captions", "json, srt or vtt"));
    }
    let lane = args
        .optional_string("lane")?
        .map(|id| {
            id.parse::<LaneId>()
                .ok()
                .filter(|id| {
                    p.lanes
                        .iter()
                        .any(|lane| lane.id == *id && lane.kind == LaneKind::Captions)
                })
                .ok_or_else(|| bad("lane", "a caption lane ID from project.info"))
        })
        .transpose()?;
    let mut cues = Vec::new();
    for &slot in p.items.order() {
        let data = p.items.data(slot).unwrap();
        if !matches!(data.kind, ItemKind::Caption { .. }) || lane.is_some_and(|id| id != data.lane)
        {
            continue;
        }
        let id = p.items.id(slot).unwrap();
        let interval = span(p, TimelineId::Item(id))?;
        let [from, to] = window.unwrap_or([interval.start, interval.end]);
        let (start, end) = (interval.start.max(from), interval.end.min(to));
        if start >= end {
            continue;
        }
        cues.push((
            start - window.map_or(0, |[from, _]| from),
            end - window.map_or(0, |[from, _]| from),
            id,
            data.lane,
            timeline::tokens(p, slot)?,
        ));
    }
    cues.sort_by_key(|cue| (cue.0, cue.2.serial()));
    if format == "json" {
        let count = p.transcripts.len();
        let signature = if count == 1 {
            hash(p.transcripts[0].hash)
        } else {
            Json::Null
        };
        let lanes = p
            .lanes
            .iter()
            .filter(|l| l.kind == LaneKind::Captions && lane.is_none_or(|id| id == l.id))
            .map(|lane| {
                object(vec![
                    ("id", display(&lane.id)),
                    ("name", text(&lane.name)),
                    ("kind", text(lane_kind(lane.kind))),
                ])
            })
            .collect();
        let cues = cues
            .into_iter()
            .map(|(from, to, id, lane, words)| {
                extend(
                    range(from, to),
                    vec![
                        ("id", display(&id)),
                        ("lane", display(&lane)),
                        ("text", Json::String(timeline::caption_text(&words))),
                        ("words", timeline::caption_json(&words)),
                    ],
                )
            })
            .collect();
        return Ok(object(vec![
            ("revision", rev(p.rev)),
            ("hash", signature),
            ("lanes", Json::Array(lanes)),
            ("cues", Json::Array(cues)),
        ]));
    }
    use std::fmt::Write;
    let mut out = if format == "vtt" {
        String::from("WEBVTT\n\n")
    } else {
        String::new()
    };
    let separator = if format == "srt" { ',' } else { '.' };
    for (index, (from, to, _, _, words)) in cues.iter().enumerate() {
        if format == "srt" {
            writeln!(out, "{}", index + 1).unwrap();
        }
        let sentence = timeline::caption_text(words);
        let sentence = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
        let sentence = if format == "vtt" {
            sentence
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        } else {
            sentence
        };
        writeln!(
            out,
            "{} --> {}\n{sentence}\n",
            timestamp(*from, p.fps, separator),
            timestamp(*to, p.fps, separator)
        )
        .unwrap();
    }
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("format", text(format)),
        ("cues", count(cues.len())),
        ("text", Json::String(out)),
    ]))
}

fn timestamp(at: Frame, fps: u32, separator: char) -> String {
    let fps = i64::from(fps);
    let millis = (i64::from(at) * 1000 + fps / 2) / fps;
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1000 % 60,
        millis % 1000
    )
}

pub(super) fn audio(store: &Store, args: &Json) -> Result<Json, StoreError> {
    let p = store.project();
    let (id, slot) = clip(p, args)?;
    let clock = ClipClock::new(p, slot)?;
    let data = p.clips.data(slot).unwrap();
    let components = clip_media(p, slot)
        .map(|media| {
            let file = media_path(store, media);
            let dir = store.path().join("takes").join(media.id.to_string());
            media
                .probe
                .audio
                .iter()
                .map(|track| {
                    let (enabled, gain) = match track.name.as_str() {
                        "mic" => (data.enabled.mic.unwrap_or(true), data.volume.mic),
                        "system" => (data.enabled.system.unwrap_or(true), data.volume.system),
                        _ => (true, 1.0),
                    };
                    let component =
                        media
                            .take
                            .as_ref()
                            .and_then(|take| match track.name.as_str() {
                                "mic" => take.mic.as_deref(),
                                "system" => take.system.as_deref(),
                                _ => None,
                            });
                    let missing =
                        component.map_or_else(|| !file.is_file(), |name| !dir.join(name).is_file());
                    // Imported files are measured at import. ponytail: recorded take components
                    // stay unmeasured until a take's loudness matters to an agent (normalize).
                    let lufs = component
                        .is_none()
                        .then_some(media.probe.loudness)
                        .flatten();
                    object(vec![
                        ("component", text(&track.name)),
                        ("enabled", Json::Bool(enabled)),
                        ("volume", Json::Float(gain)),
                        ("integratedLufs", lufs.map_or(Json::Null, Json::Float)),
                        ("peakDbfs", Json::Null),
                        (
                            "state",
                            text(match (missing, lufs) {
                                (true, _) => "offline",
                                (false, Some(_)) => "measured",
                                (false, None) => "unmeasured",
                            }),
                        ),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let mut pauses = Vec::new();
    if let Some(transcript) = clip_transcript(p, slot) {
        let mut words: Vec<_> = transcript.words.iter().collect();
        words.sort_by_key(|word| word.from);
        let minimum = f64::from(p.fps) * 0.5;
        let mut last = clock.timing.source;
        for word in words {
            let from = f64::from(last);
            let to = f64::from(word.from);
            if clock.has(from)
                && to <= clock.source_end()
                && (to - from) / clock.rate >= minimum
                && last != clock.timing.source
            {
                let (from, to) = clock.interval(from, to);
                pauses.push(extend(range(from, to), vec![("basis", text("transcript"))]));
            }
            last = last.max(word.to);
        }
    }
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("clip", display(&id)),
        ("components", Json::Array(components)),
        ("silences", Json::Array(pauses)),
    ]))
}

pub(super) fn events(dir: &Path, p: &Project, args: &Json) -> Result<Json, StoreError> {
    let (id, slot) = clip(p, args)?;
    let media = clip_media(p, slot)
        .filter(|media| media.kind == MediaKind::Take)
        .ok_or_else(not_a_take)?;
    let clock = ClipClock::new(p, slot)?;
    let (from, to) = timeline::window(args, clock.timing.end().unwrap())?;
    let mut cursor = Vec::new();
    let mut clicks = Vec::new();
    let mut keys = Vec::new();
    let mut caret = Vec::new();
    let mut key_index = 0;
    if let Some(events_name) = media.take.as_ref().and_then(|take| take.events.as_deref()) {
        let dir = dir.join("takes").join(media.id.to_string());
        for_each_event(&dir, events_name, &mut |event| {
            let kind = event
                .field("k")
                .and_then(Json::as_str)
                .ok_or_else(events_format)?;
            let at = event
                .field("t")
                .and_then(Json::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or_else(events_format)?;
            let source = at * media.probe.fps;
            let shown = clock.has(source) && (from..to).contains(&clock.start(source));
            let index = key_index;
            if kind == "key" {
                key_index += 1;
            }
            if !shown {
                return Ok(());
            }
            let mut fields = vec![
                ("kind", text(kind)),
                ("frame", frame(clock.start(source))),
                ("sourceFrame", Json::Signed(source.round() as i64)),
            ];
            match kind {
                "move" | "drag" | "up" | "down" | "scroll" | "cursor" => {
                    for axis in ["x", "y"] {
                        if let Some(coord) = event.field(axis).and_then(Json::as_f64) {
                            fields.push((axis, Json::Float(coord)));
                        }
                    }
                    if kind == "down" {
                        if let Some(button) = event.field("b").and_then(Json::as_u64) {
                            fields.push(("button", Json::Unsigned(button)));
                        }
                        clicks.push(object(fields));
                    } else {
                        cursor.push(object(fields));
                    }
                }
                "key" => {
                    let data = p.clips.data(slot).unwrap();
                    let mods = event.field("mods").and_then(Json::as_u64).unwrap_or(0);
                    let category = if mods & splice_eval::activity::SHORTCUT_MODIFIERS != 0 {
                        "shortcut"
                    } else {
                        "typing"
                    };
                    fields.extend([
                        ("index", count(index)),
                        ("category", text(category)),
                        (
                            "hidden",
                            Json::Bool(data.hidden_keys.contains(&(index as u32))),
                        ),
                        ("redacted", Json::Bool(true)),
                        ("text", Json::Null),
                    ]);
                    keys.push(object(fields));
                }
                // The text insertion point (T45.4): where typing lands, from Accessibility.
                "caret" => {
                    for axis in ["x", "y", "w", "h"] {
                        let value = event
                            .field(axis)
                            .and_then(Json::as_f64)
                            .filter(|v| v.is_finite())
                            .ok_or_else(events_format)?;
                        fields.push((axis, Json::Float(value)));
                    }
                    caret.push(object(fields));
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("clip", display(&id)),
        ("cursor", Json::Array(cursor)),
        ("clicks", Json::Array(clicks)),
        ("keys", Json::Array(keys)),
        ("caret", Json::Array(caret)),
    ]))
}

/// Clicks (button downs) and key presses in a take's event log (record.stop's take summary).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn input_counts(dir: &Path, name: &str) -> Result<(u64, u64), StoreError> {
    let (mut clicks, mut keys) = (0, 0);
    for_each_event(dir, name, &mut |event| {
        match event.field("k").and_then(Json::as_str) {
            Some("down") => clicks += 1,
            Some("key") => keys += 1,
            Some(_) => {}
            None => return Err(events_format()),
        }
        Ok(())
    })?;
    Ok((clicks, keys))
}

fn events_format() -> StoreError {
    failure(
        "BAD_PACKAGE",
        "The take event log is invalid.",
        "Check the take's events.json against docs/FORMAT.md.",
    )
}

/// Streams a take's event log (T15.3): each event of the `events` array is parsed and visited
/// on its own, so memory stays flat however long the take is.
fn for_each_event(
    dir: &Path,
    name: &str,
    visit: &mut dyn FnMut(&Json) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    use std::io::BufRead;
    let offline = || {
        failure(
            "MEDIA_OFFLINE",
            "The take's event file is offline.",
            "Restore the take's event file under its takes directory.",
        )
    };
    let path = fs::canonicalize(dir.join(name)).map_err(|_| offline())?;
    if !path.starts_with(dir) || !path.is_file() {
        return Err(events_format());
    }
    let mut reader =
        std::io::BufReader::with_capacity(65536, fs::File::open(path).map_err(|_| offline())?);
    // Skip to the events array: the key, then `:` and `[` with any JSON whitespace around.
    let key = b"\"events\"";
    let (mut matched, mut colon, mut found) = (0, false, false);
    while !found {
        let buf = reader.fill_buf().map_err(|_| events_format())?;
        if buf.is_empty() {
            return Err(events_format());
        }
        let mut used = 0;
        for &byte in buf {
            used += 1;
            if matched == key.len() {
                match byte {
                    b' ' | b'\t' | b'\n' | b'\r' => continue,
                    b':' if !colon => colon = true,
                    b'[' if colon => {
                        found = true;
                        break;
                    }
                    _ => (matched, colon) = (0, false),
                }
                continue;
            }
            matched = if byte == key[matched] {
                matched + 1
            } else if byte == key[0] {
                1
            } else {
                0
            };
        }
        reader.consume(used);
    }
    // Objects one at a time: depth, strings and escapes decide where each ends.
    let mut object = String::new();
    let (mut depth, mut in_string, mut escaped) = (0u32, false, false);
    let mut bytes = Vec::with_capacity(256);
    loop {
        let buf = reader.fill_buf().map_err(|_| events_format())?;
        if buf.is_empty() {
            return Err(events_format());
        }
        let mut used = 0;
        let mut done = false;
        for &byte in buf {
            used += 1;
            if depth == 0 {
                match byte {
                    b'{' => {
                        depth = 1;
                        bytes.clear();
                        bytes.push(byte);
                    }
                    b']' => {
                        done = true;
                        break;
                    }
                    _ => {}
                }
                continue;
            }
            bytes.push(byte);
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        object.clear();
                        object.push_str(std::str::from_utf8(&bytes).map_err(|_| events_format())?);
                        visit(&splice_json::parse(&object).map_err(|_| events_format())?)?;
                    }
                }
                _ => {}
            }
        }
        reader.consume(used);
        if done {
            return Ok(());
        }
    }
}

/// A take's activity over `window` (media seconds), from its event log and transcript words
/// (T41.1). Only event kinds, times and modifier flags are read; typed characters never are.
pub(super) fn activity(
    dir: &Path,
    p: &Project,
    media: &Media,
    window: [f64; 2],
) -> Result<splice_eval::activity::Activity, StoreError> {
    use splice_eval::activity::{Event, Input};
    if media.kind != MediaKind::Take {
        return Err(not_a_take());
    }
    let mut events = Vec::new();
    if let Some(name) = media.take.as_ref().and_then(|take| take.events.as_deref()) {
        let dir = dir.join("takes").join(media.id.to_string());
        for_each_event(&dir, name, &mut |event| {
            let kind = event
                .field("k")
                .and_then(Json::as_str)
                .ok_or_else(events_format)?;
            let t = event
                .field("t")
                .and_then(Json::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or_else(events_format)?;
            let input = match kind {
                "move" | "drag" => Input::Pointer,
                "down" => Input::Press,
                "up" => Input::Release,
                "scroll" => Input::Scroll,
                "flags" => Input::Shortcut,
                "key" => {
                    let mods = event.field("mods").and_then(Json::as_u64).unwrap_or(0);
                    if mods & splice_eval::activity::SHORTCUT_MODIFIERS != 0 {
                        Input::Shortcut
                    } else {
                        Input::Typing
                    }
                }
                // Cursor image changes are not the person doing anything.
                _ => return Ok(()),
            };
            events.push(Event { t, input });
            Ok(())
        })?;
    }
    let fps = media.probe.fps.max(1.0);
    let speech: Vec<[f64; 2]> = p
        .transcripts
        .iter()
        .filter(|transcript| transcript.media == media.id)
        .flat_map(|transcript| &transcript.words)
        .map(|word| [f64::from(word.from) / fps, f64::from(word.to) / fps])
        .collect();
    Ok(splice_eval::activity::activity(&events, &speech, window))
}

fn not_a_take() -> StoreError {
    failure(
        "WRONG_KIND",
        "The clip is not a recorded take.",
        "Use a take clip ID from timeline.snapshot.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A take log written with JSON whitespace (`"events": [`, as most tools write it) parses
    /// like the recorder's compact form (found by the silent-demo-short eval).
    #[test]
    fn event_logs_may_use_json_whitespace() {
        let dir = crate::tests::TestDirectory::new("events-whitespace");
        let dir = fs::canonicalize(&dir.0).unwrap();
        for (name, text) in [
            (
                "compact.json",
                r#"{"events":[{"t":1,"k":"move","x":0.5,"y":0.5}]}"#,
            ),
            (
                "pretty.json",
                "{\n  \"source\": {\"rect\": {\"width\": 100}},\n  \"events\" : [\n    {\"t\": 1, \"k\": \"move\", \"x\": 0.5, \"y\": 0.5},\n    {\"t\": 2, \"k\": \"key\", \"keyCode\": 4}\n  ]\n}",
            ),
        ] {
            fs::write(dir.join(name), text).unwrap();
            let mut seen = 0;
            for_each_event(&dir, name, &mut |_| {
                seen += 1;
                Ok(())
            })
            .unwrap();
            assert!(seen >= 1, "{name}");
        }
    }
}
