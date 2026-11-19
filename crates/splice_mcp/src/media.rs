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
use std::{fs, io::Read, path::Path};

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
        return captions(p, args, format);
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
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("hash", hash(transcript.hash)),
        ("clip", display(&id)),
        ("language", text(&transcript.language)),
        ("words", Json::Array(words)),
        ("fillers", Json::Array(fillers)),
        ("nextCursor", next_cursor('w', end, total)),
    ]))
}

fn filler(text: &str) -> bool {
    let word = text.trim_matches(|ch: char| ch.is_ascii_punctuation());
    word.eq_ignore_ascii_case("um") || word.eq_ignore_ascii_case("uh")
}

fn no_transcript() -> StoreError {
    failure(
        "NOT_FOUND",
        "No transcript is available for this clip.",
        "Call project.info for transcript state or pass a clip with a transcript.",
    )
}

pub(super) fn captions(p: &Project, args: &Json, format: &str) -> Result<Json, StoreError> {
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
        cues.push((
            interval.start,
            interval.end,
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

pub(super) fn events(store: &Store, args: &Json) -> Result<Json, StoreError> {
    let p = store.project();
    let (id, slot) = clip(p, args)?;
    let media = clip_media(p, slot)
        .filter(|media| media.kind == MediaKind::Take)
        .ok_or_else(|| {
            failure(
                "WRONG_KIND",
                "The clip is not a recorded take.",
                "Use a take clip ID from timeline.snapshot.",
            )
        })?;
    let clock = ClipClock::new(p, slot)?;
    let (from, to) = timeline::window(args, clock.timing.end().unwrap())?;
    let mut cursor = Vec::new();
    let mut clicks = Vec::new();
    let mut keys = Vec::new();
    let mut key_index = 0;
    if let Some(events_name) = media.take.as_ref().and_then(|take| take.events.as_deref()) {
        let dir = store.path().join("takes").join(media.id.to_string());
        let log = read_event_log(&dir, events_name)?;
        let events = log
            .field("events")
            .and_then(Json::as_array)
            .ok_or_else(events_format)?;
        for event in events {
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
                continue;
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
                    let category = if mods & ((1 << 18) | (1 << 19) | (1 << 20)) != 0 {
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
                _ => {}
            }
        }
    }
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("clip", display(&id)),
        ("cursor", Json::Array(cursor)),
        ("clicks", Json::Array(clicks)),
        ("keys", Json::Array(keys)),
    ]))
}

fn events_format() -> StoreError {
    failure(
        "BAD_PACKAGE",
        "The take event log is invalid.",
        "Check the take's events.json against docs/FORMAT.md.",
    )
}

fn read_event_log(dir: &Path, name: &str) -> Result<Json, StoreError> {
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
    let mut buf = String::new();
    // ponytail: read one bounded capture log until long takes justify a streaming time index.
    let limit = 16 * 1024 * 1024;
    fs::File::open(path)
        .map_err(|_| offline())?
        .take(limit + 1)
        .read_to_string(&mut buf)
        .map_err(|_| events_format())?;
    if buf.len() as u64 > limit {
        return Err(failure(
            "OUT_OF_RANGE",
            "The take event log exceeds 16 MiB.",
            "Inspect this take in the editor until indexed event reads are available.",
        ));
    }
    splice_json::parse(&buf).map_err(|_| events_format())
}
