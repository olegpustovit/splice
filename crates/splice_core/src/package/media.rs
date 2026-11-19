use super::{PackageError, codec::*};
use splice_json::Value as Json;
use splice_model::*;

pub(super) fn encode(media: &[Media]) -> Json {
    object(vec![
        ("format", text(FORMAT)),
        (
            "media",
            Json::Array(media.iter().map(encode_media).collect()),
        ),
    ])
}

pub(super) fn decode(value: &Json) -> Result<Vec<Media>, PackageError> {
    check_version(value)?;
    shape(value, &["format", "media"])?;
    value.array("media")?.iter().map(decode_media).collect()
}

pub(super) fn encode_media(media: &Media) -> Json {
    let mut fields = vec![
        ("id", display(&media.id)),
        (
            "kind",
            text(match media.kind {
                MediaKind::Video => "video",
                MediaKind::Audio => "audio",
                MediaKind::Image => "image",
                MediaKind::Take => "take",
                MediaKind::Compound => "compound",
            }),
        ),
        ("path", text(&media.path)),
    ];
    push_text(&mut fields, "bookmark", &media.bookmark);
    fields.extend([
        ("size", Json::Unsigned(media.size)),
        ("mtime", Json::Signed(media.mtime)),
        ("probe", encode_probe(&media.probe)),
    ]);
    if let Some(take) = &media.take {
        let mut files = Vec::new();
        for (name, path) in [
            ("screen", &take.screen),
            ("camera", &take.camera),
            ("mic", &take.mic),
            ("system", &take.system),
            ("events", &take.events),
            ("cursors", &take.cursors),
        ] {
            push_text(&mut files, name, path);
        }
        fields.push(("components", object(files)));
    }
    object(fields)
}

pub(super) fn decode_media(value: &Json) -> Result<Media, PackageError> {
    shape(
        value,
        &[
            "id",
            "kind",
            "path",
            "bookmark",
            "size",
            "mtime",
            "probe",
            "components",
        ],
    )?;
    let take = if let Some(value) = value.optional("components") {
        shape(
            value,
            &["screen", "camera", "mic", "system", "events", "cursors"],
        )?;
        Some(TakeComponents {
            screen: optional_string(value, "screen")?,
            camera: optional_string(value, "camera")?,
            mic: optional_string(value, "mic")?,
            system: optional_string(value, "system")?,
            events: optional_string(value, "events")?,
            cursors: optional_string(value, "cursors")?,
        })
    } else {
        None
    };
    Ok(Media {
        id: value
            .string("id")?
            .parse()
            .map_err(|_| bad_id("media.id"))?,
        kind: match value.string("kind")? {
            "video" => MediaKind::Video,
            "audio" => MediaKind::Audio,
            "image" => MediaKind::Image,
            "take" => MediaKind::Take,
            "compound" => MediaKind::Compound,
            _ => return Err(bad("media.kind", "video, audio, image, take or compound")),
        },
        path: value.string("path")?.into(),
        bookmark: optional_string(value, "bookmark")?,
        size: u64_field(value, "size")?,
        mtime: value
            .required("mtime")?
            .as_i64()
            .ok_or_else(|| bad("mtime", "integer seconds"))?,
        probe: decode_probe(value.required("probe")?)?,
        take,
    })
}

fn encode_probe(probe: &Probe) -> Json {
    let mut fields = vec![
        ("fps", Json::Float(probe.fps)),
        ("frames", frame_json(probe.frames)),
        ("width", Json::Unsigned(u64::from(probe.width))),
        ("height", Json::Unsigned(u64::from(probe.height))),
        (
            "audio",
            Json::Array(
                probe
                    .audio
                    .iter()
                    .map(|track| match track.channels {
                        Some(channels) => object(vec![
                            ("name", text(&track.name)),
                            ("channels", Json::Unsigned(u64::from(channels))),
                        ]),
                        None => text(&track.name),
                    })
                    .collect(),
            ),
        ),
    ];
    push_text(&mut fields, "codec", &probe.codec);
    if let Some(lufs) = probe.loudness {
        fields.push(("lufs", Json::Float(lufs)));
    }
    object(fields)
}

fn decode_probe(value: &Json) -> Result<Probe, PackageError> {
    shape(
        value,
        &["fps", "frames", "width", "height", "audio", "codec", "lufs"],
    )?;
    let mut audio = Vec::new();
    for track in value.array("audio")? {
        if let Some(name) = track.as_str() {
            audio.push(AudioTrack {
                name: name.into(),
                channels: None,
            });
        } else {
            shape(track, &["name", "channels"])?;
            audio.push(AudioTrack {
                name: track.string("name")?.into(),
                channels: Some(u32_field(track, "channels")?),
            });
        }
    }
    Ok(Probe {
        fps: number(value, "fps")?,
        frames: frame(value, "frames")?,
        width: u32_field(value, "width")?,
        height: u32_field(value, "height")?,
        audio,
        codec: optional_string(value, "codec")?,
        loudness: value
            .field("lufs")
            .map(|_| number(value, "lufs"))
            .transpose()?,
    })
}

pub(super) fn encode_transcript(transcript: &Transcript) -> Json {
    object(vec![
        ("hash", hash_text(transcript.hash)),
        ("language", text(&transcript.language)),
        (
            "words",
            Json::Array(
                transcript
                    .words
                    .iter()
                    .map(|word| {
                        let mut fields = vec![
                            ("id", display(&word.id)),
                            ("text", text(&word.text)),
                            ("from", frame_json(word.from)),
                            ("to", frame_json(word.to)),
                        ];
                        if let Some(confidence) = word.confidence {
                            fields.push(("conf", Json::Float(confidence)));
                        }
                        object(fields)
                    })
                    .collect(),
            ),
        ),
    ])
}

pub(super) fn decode_transcript(media: MediaId, value: &Json) -> Result<Transcript, PackageError> {
    shape(value, &["hash", "language", "words"])?;
    let mut words = Vec::new();
    for word in value.array("words")? {
        shape(word, &["id", "text", "from", "to", "conf"])?;
        words.push(Word {
            id: word.string("id")?.parse().map_err(|_| bad_id("word.id"))?,
            text: word.string("text")?.into(),
            from: frame(word, "from")?,
            to: frame(word, "to")?,
            confidence: word
                .optional("conf")
                .map(|_| number(word, "conf"))
                .transpose()?,
        });
    }
    Ok(Transcript {
        media,
        hash: hash(value, "hash")?,
        language: value.string("language")?.into(),
        words,
    })
}
