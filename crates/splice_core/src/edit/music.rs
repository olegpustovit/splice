//! `music.add` (T45.3): a library track imported as media, laid on an audio lane to the
//! timeline end (looped or cut with a fade-out), ducked under speech and leveled. `splice mcp`
//! fills the file and credit fields from the music library, so the op replays without it.
use super::time::{item_slot, item_span, media, rounded};
use super::*;
use crate::commands::{FrameArg, MusicAdd, MusicFit};

/// Ducking `music.add` applies: about −12 dB under speech, `audio.set`'s attack and release.
const DUCK: Duck = Duck {
    amount: 0.25,
    attack: 8,
    release: 15,
};

/// music.add's loudness target when the call names none (social platforms).
const DEFAULT_LUFS: f64 = -14.0;

impl Editor {
    pub(super) fn add_music(
        &mut self,
        music: MusicAdd,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<Outcome, CommandError> {
        let license = music.license.as_deref().map(str::trim).unwrap_or("");
        if !music.allow_unlicensed
            && (license.is_empty() || license.eq_ignore_ascii_case("unknown"))
        {
            return Err(CommandError::rule(
                "UNLICENSED",
                "license",
                Some(Json::String(
                    music.license.clone().unwrap_or_else(|| "unknown".into()),
                )),
                "a known license (e.g. CC0-1.0) from the library's tracks.json",
                "Pick a licensed track (commands.describe {music:{}} lists each license), add \
                 \"license\" for this file to the library's tracks.json, or pass \
                 allowUnlicensed:true if you hold the rights.",
            ));
        }
        let fps = self.project.fps;
        let (seconds, measured) = (music.seconds, music.loudness);
        // Without a media backend (Linux) the catalog's length stands in for the probe; the
        // descriptor is recorded with the batch either way, so replay never opens the file.
        let mut import = |path: &str| match load(path) {
            Err(error) if error.message == UNPROBED => match seconds {
                Some(seconds) => Ok(Media {
                    id: MediaId::from_serial(0),
                    kind: MediaKind::Audio,
                    path: path.into(),
                    bookmark: None,
                    size: 0,
                    mtime: 0,
                    probe: Probe {
                        fps: f64::from(fps),
                        frames: rounded((seconds * f64::from(fps)).round())?,
                        width: 0,
                        height: 0,
                        audio: vec![AudioTrack {
                            name: "audio".into(),
                            channels: None,
                        }],
                        codec: None,
                        loudness: measured,
                    },
                    take: None,
                }),
                None => Err(error),
            },
            other => other,
        };
        let media_id = self.import_path(music.path.clone(), &mut import)?;
        let loudness = media(&self.project, media_id)?.probe.loudness.or(measured);
        let volume = match (music.volume, loudness) {
            (Some(volume), _) => volume,
            (None, Some(loudness)) => match music.normalize {
                Some(target) => super::tier_b::normalize_gain(target, loudness)?,
                // The default is best effort: a quiet track gets the ceiling and says so.
                None => super::tier_b::normalize_gain(DEFAULT_LUFS, loudness).unwrap_or(2.0),
            },
            (None, None) => {
                return Err(error(
                    "LOUDNESS_UNKNOWN",
                    &format!("{} has no measured loudness.", music.path),
                    "Pass volume (0–2) to set the level yourself, or add \"loudness\" (LUFS) for \
                     this file to the library's tracks.json; macOS measures it at import.",
                ));
            }
        };
        let at = music
            .at
            .as_ref()
            .map(|at| self.frame(at))
            .transpose()?
            .unwrap_or(0);
        if at < 0 {
            return Err(CommandError::bad("at", "a nonnegative frame"));
        }
        let end = self.spine_end()?;
        if at > end - MIN_LENGTH {
            return Err(CommandError::rule(
                "PAST_END",
                "at",
                Some(Json::Signed(i64::from(at))),
                &format!("a start at least {MIN_LENGTH} frames before the spine end, frame {end}"),
                "The timeline ends with its last clip: start the music earlier, or add clips first.",
            ));
        }
        let lane = match &music.lane {
            Some(lane) => lane.clone(),
            None => Target::Lane(self.free_audio_lane(at, end)?),
        };
        let mut created = Vec::new();
        let mut start = at;
        let mut cut;
        loop {
            let outcome = self.add_sound(media_id, Some(&FrameArg::Frame(start)), Some(&lane))?;
            let Some(Target::Timeline(TimelineId::Item(id))) = outcome.primary else {
                unreachable!("add_sound creates an item")
            };
            let slot = item_slot(&self.project, id)?;
            let timing = self.project.items.timing_mut(slot).expect("live item");
            let track = timing.length;
            timing.length = track.min(end - start);
            cut = timing.length < track;
            let length = timing.length;
            let data = self.project.items.data_mut(slot).expect("live item");
            if let ItemKind::Sound(sound) = &mut data.kind {
                sound.volume = volume;
                sound.duck = music.duck.then_some(DUCK);
            }
            created.push(Target::Timeline(TimelineId::Item(id)));
            start += length;
            if music.fit == MusicFit::Trim || end - start < MIN_LENGTH {
                break;
            }
        }
        // The timeline end cuts the last piece: fade it out rather than stop mid-phrase.
        let last = match created.last() {
            Some(Target::Timeline(TimelineId::Item(id))) => item_slot(&self.project, *id)?,
            _ => unreachable!("one piece at least"),
        };
        let fade = if cut {
            music
                .fade_out
                .unwrap_or_else(|| Frame::try_from(fps * 2).unwrap_or(60))
        } else {
            0
        };
        if fade > 0 {
            let length = self.project.items.timing(last).expect("live item").length;
            let appearance = &mut self
                .project
                .items
                .data_mut(last)
                .expect("live item")
                .appearance;
            appearance.fade_out = fade.min(length - appearance.fade_in);
        }
        let name = music.title.as_deref().unwrap_or(&music.path);
        let level = match music.volume {
            Some(volume) => format!("volume {volume:.2}"),
            None => match (music.normalize, loudness) {
                (None, Some(loudness))
                    if volume >= 2.0 && loudness + 20.0 * 2f64.log10() < DEFAULT_LUFS - 0.05 =>
                {
                    format!(
                        "volume 2.00 (the gain ceiling; the track measures {loudness:.1} LUFS, too quiet for {DEFAULT_LUFS} LUFS)"
                    )
                }
                (target, _) => format!("normalized to {} LUFS", target.unwrap_or(DEFAULT_LUFS)),
            },
        };
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Added music \"{name}\" ({}) from frame {at} to {start} in {} piece(s), {level}{}{}",
                if license.is_empty() {
                    "unknown license"
                } else {
                    license
                },
                created.len(),
                if music.duck {
                    ", ducked under speech"
                } else {
                    ""
                },
                if fade > 0 {
                    format!(", fading out over {fade} frames")
                } else {
                    String::new()
                },
            )),
            primary: created.first().cloned(),
            created,
        })
    }

    /// The last audio lane with nothing on `[from, to)`, or a new one.
    fn free_audio_lane(&mut self, from: Frame, to: Frame) -> Result<LaneId, CommandError> {
        for lane in self.project.lanes.iter().rev() {
            if lane.kind != LaneKind::Audio {
                continue;
            }
            let mut busy = false;
            for &slot in self.project.items.order() {
                if self.project.items.data(slot).expect("live item").lane == lane.id {
                    let span = item_span(&self.project, slot)?;
                    busy |= span.start < to && span.end > from;
                }
            }
            if !busy {
                return Ok(lane.id);
            }
        }
        self.new_lane(LaneKind::Audio, None)
    }
}
