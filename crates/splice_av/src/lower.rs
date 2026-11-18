use crate::{Error, TimedText};
use apple_sys::composition::{
    AudioRamp, Builder, Composition, Source, TextLayer, Timing, VideoLayer,
};
use splice_eval::{Audio, Background, CaptionPreset, Evaluator, Layer, Picture, Track};
use splice_model::{ClipId, Frame, Media, MediaId, MediaKind, Project, Rect, TimelineId};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Picture(TimelineId),
    Camera(ClipId),
    Audio(TimelineId, Track, bool),
    Fill(usize, u32, u32),
}

struct Lowering {
    builder: Builder,
    text: Vec<TimedText>,
    end: Frame,
    tracks: Vec<(Key, i32)>,
}

pub(super) fn build(
    project: &Project,
    package: &Path,
    evaluator: &Evaluator,
    cuts: &[Frame],
    end: Frame,
) -> Result<(Composition, Vec<TimedText>), Error> {
    let mut state = Lowering {
        builder: Builder::new(project.size, project.fps, end)?,
        text: Vec::new(),
        end,
        tracks: Vec::new(),
    };
    let black = state.fill(usize::MAX, 0, 0)?;
    let mut first = evaluator.buffer();
    let mut last = evaluator.buffer();
    let mut video = Vec::new();
    let mut audio = Vec::new();
    let [width, height] = project.size.map(f64::from);
    let bounds = [0.0, 0.0, width, height];
    for pair in cuts.windows(2) {
        let [from, to] = [pair[0], pair[1]];
        let range = [from, to];
        evaluator.frame(from, &mut first);
        evaluator.frame(to - 1, &mut last);
        if first.layers.len() != last.layers.len() || first.audio.len() != last.audio.len() {
            return Err(Error::Input(format!(
                "The render stack changes inside frames {from}–{to}."
            )));
        }
        video.clear();
        audio.clear();
        // Keep a real video track under blank, audio-only and text-only intervals.
        if !first
            .layers
            .iter()
            .any(|layer| matches!(layer, Layer::Fade { .. }))
        {
            video.push(layer(black, bounds, [1.0; 2]));
        }
        for (depth, (&a, &b)) in first.layers.iter().zip(&last.layers).enumerate() {
            match (a, b) {
                (Layer::Picture(a), Layer::Picture(b)) => {
                    let track = state.picture(a, project, package, evaluator)?;
                    video.push(VideoLayer {
                        track,
                        crop: coordinates(a.source),
                        destination: coordinates(a.destination),
                        scale: ramp(a.scale, b.scale, range),
                        opacity: opacity(a.opacity, b.opacity, range),
                        center_crop: 0,
                    });
                }
                (
                    Layer::Background {
                        bounds,
                        fill,
                        opacity: a,
                    },
                    Layer::Background { opacity: b, .. },
                ) => {
                    let [a_color, b_color] = match fill {
                        Background::None => continue,
                        Background::Solid(value) => [value; 2],
                        Background::Gradient(a, b) => [a, b],
                    };
                    let track = state.fill(depth, a_color, b_color)?;
                    video.push(layer(track, coordinates(bounds), opacity(a, b, range)));
                }
                (Layer::Fade { opacity: a }, Layer::Fade { opacity: b }) => {
                    video.push(layer(black, bounds, opacity(a, b, range)));
                }
                (
                    Layer::Camera {
                        clip,
                        media,
                        bounds,
                        ..
                    },
                    Layer::Camera { .. },
                ) => {
                    let track = state.camera(clip, media, project, package, evaluator)?;
                    // ponytail: use rectangular video crops until the M3 compositor supports masks and shadows.
                    let mut camera = layer(track, coordinates(bounds), [1.0; 2]);
                    camera.center_crop = 1;
                    video.push(camera);
                }
                (
                    Layer::Title {
                        item,
                        bounds,
                        opacity: a,
                    },
                    Layer::Title { opacity: b, .. },
                ) => {
                    if let Some(text) = evaluator.title(item) {
                        state.add_text(
                            range,
                            TextLayer {
                                text: text.into(),
                                bounds: coordinates(bounds),
                                font: bounds.h * 0.6,
                                opacity: opacity(a, b, range).map(|v| v as f32),
                                color: 0xffffff,
                            },
                        )?;
                    }
                }
                (
                    Layer::Caption {
                        item,
                        word,
                        opacity: a,
                        style,
                    },
                    Layer::Caption { opacity: b, .. },
                ) => {
                    if let Some(words) = evaluator.caption_words(item) {
                        let count = match style.preset {
                            CaptionPreset::Pop | CaptionPreset::Typewriter => {
                                word.map_or(0, |i| i + 1)
                            }
                            _ => words.len(),
                        };
                        let mut text = String::new();
                        for word in &words[..count] {
                            if !text.is_empty() {
                                text.push(' ');
                            }
                            text.push_str(&word.text);
                            if let Some(emoji) = &word.emoji {
                                text.push_str(emoji);
                            }
                        }
                        if !text.is_empty() {
                            let [top, bottom, left, right] = style.safe;
                            let font = (width * style.size).min(height * 0.15);
                            let content_height = font * 2.5;
                            let y = (height * style.position - content_height / 2.0).clamp(
                                height * top,
                                (height * (1.0 - bottom) - content_height).max(height * top),
                            );
                            state.add_text(
                                range,
                                TextLayer {
                                    text,
                                    bounds: [
                                        width * left,
                                        y,
                                        width * (1.0 - left - right),
                                        content_height,
                                    ],
                                    font,
                                    opacity: opacity(a, b, range).map(|v| v as f32),
                                    color: style.color,
                                },
                            )?;
                        }
                    }
                }
                // ponytail: render recorded keystrokes when T8 supplies the event stream.
                (Layer::Keystrokes { .. }, Layer::Keystrokes { .. }) => {}
                _ => {
                    return Err(Error::Input(format!(
                        "The render layers reorder inside frames {from}–{to}."
                    )));
                }
            }
        }
        for (&a, &b) in first.audio.iter().zip(&last.audio) {
            state.audio([a, b], range, &mut audio, project, package, evaluator)?;
        }
        state.builder.range(range, &video, &audio)?;
    }
    Ok((state.builder.finish()?, state.text))
}

impl Lowering {
    fn add_text(&mut self, range: [Frame; 2], layer: TextLayer) -> Result<(), Error> {
        self.builder.text(range, &layer)?;
        self.text.push(TimedText { range, layer });
        Ok(())
    }
    fn cached(&self, key: Key) -> Option<i32> {
        // ponytail: use linear lookup until it shows up in composition rebuild profiles.
        self.tracks
            .iter()
            .find_map(|&(candidate, track)| (candidate == key).then_some(track))
    }

    fn media(project: &Project, id: MediaId) -> Result<&Media, Error> {
        project
            .media
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| Error::Input(format!("Missing media descriptor {id}.")))
    }

    fn timing(
        fps: u32,
        evaluator: &Evaluator,
        item: TimelineId,
        media: &Media,
    ) -> Result<Timing, Error> {
        let span = evaluator
            .source_span(item)
            .ok_or_else(|| Error::Input(format!("Missing source timing for {item}.")))?;
        let source_fps = media.probe.fps;
        Ok(Timing {
            timeline: [span.start, span.end],
            source_start: span.source / source_fps,
            source_limit: f64::from(media.probe.frames) / source_fps,
            speed: span.step * f64::from(fps) / source_fps,
            source_frame_duration: 1.0 / source_fps,
        })
    }

    fn fill(&mut self, depth: usize, a: u32, b: u32) -> Result<i32, Error> {
        let key = Key::Fill(depth, a, b);
        if let Some(track) = self.cached(key) {
            return Ok(track);
        }
        let timing = Timing {
            timeline: [0, self.end],
            source_start: 0.0,
            source_limit: 1.0,
            speed: 1.0,
            source_frame_duration: 1.0,
        };
        let track = self.builder.source(Source::Color(a, b), &timing)?;
        self.tracks.push((key, track));
        Ok(track)
    }

    fn picture(
        &mut self,
        picture: Picture,
        project: &Project,
        package: &Path,
        evaluator: &Evaluator,
    ) -> Result<i32, Error> {
        let key = Key::Picture(picture.item);
        if let Some(track) = self.cached(key) {
            return Ok(track);
        }
        let media = Self::media(project, picture.media)?;
        let timing = Self::timing(project.fps, evaluator, picture.item, media)?;
        let path = media_path(
            package,
            media,
            media.take.as_ref().and_then(|take| take.screen.as_deref()),
        );
        let source = if media.kind == MediaKind::Image {
            Source::Still(path)
        } else {
            Source::Video(path)
        };
        let track = self.builder.source(source, &timing)?;
        self.tracks.push((key, track));
        Ok(track)
    }

    fn camera(
        &mut self,
        clip: ClipId,
        id: MediaId,
        project: &Project,
        package: &Path,
        evaluator: &Evaluator,
    ) -> Result<i32, Error> {
        let key = Key::Camera(clip);
        if let Some(track) = self.cached(key) {
            return Ok(track);
        }
        let media = Self::media(project, id)?;
        let timing = Self::timing(project.fps, evaluator, TimelineId::Clip(clip), media)?;
        let component = media
            .take
            .as_ref()
            .and_then(|take| take.camera.as_deref())
            .ok_or_else(|| Error::Input(format!("{id} has no camera component.")))?;
        let path = media_path(package, media, Some(component));
        let track = self.builder.source(Source::Video(path), &timing)?;
        self.tracks.push((key, track));
        Ok(track)
    }

    fn audio(
        &mut self,
        [a, b]: [Audio; 2],
        range: [Frame; 2],
        out: &mut Vec<AudioRamp>,
        project: &Project,
        package: &Path,
        evaluator: &Evaluator,
    ) -> Result<(), Error> {
        let media = Self::media(project, a.media)?;
        let timing = Self::timing(project.fps, evaluator, a.item, media)?;
        let take = media.take.as_ref();
        let (path, index) = match a.track {
            Track::Imported(index) => (package.join(&media.path), index),
            Track::Mic | Track::System => {
                let (name, file) = if a.track == Track::Mic {
                    ("mic", take.and_then(|take| take.mic.as_deref()))
                } else {
                    ("system", take.and_then(|take| take.system.as_deref()))
                };
                if let Some(file) = file {
                    (media_path(package, media, Some(file)), 0)
                } else {
                    let index = media
                        .probe
                        .audio
                        .iter()
                        .position(|track| track.name == name)
                        .ok_or_else(|| {
                            Error::Input(format!("{} has no {name} audio track.", media.id))
                        })?;
                    (package.join(&media.path), index as u32)
                }
            }
        };
        let peak = evaluator
            .audio_gain(a.item, a.track)
            .ok_or_else(|| Error::Input(format!("Missing audio gain for {}.", a.item)))?;
        if peak > 2.0 {
            return Err(Error::Input(format!(
                "Audio gain exceeds 2.0 on {}.",
                a.item
            )));
        }
        let gain =
            ramp(a.gain, b.gain, range).map(|gain| (gain / peak.max(1.0)).clamp(0.0, 1.0) as f32);
        let track = self.audio_track(Key::Audio(a.item, a.track, false), &path, index, &timing)?;
        out.push(AudioRamp { track, gain });
        // AVFoundation supports up to unity per input. A second copy contributes the surplus.
        if peak > 1.0 {
            let track =
                self.audio_track(Key::Audio(a.item, a.track, true), &path, index, &timing)?;
            let gain = gain.map(|gain| gain * (peak - 1.0) as f32);
            out.push(AudioRamp { track, gain });
        }
        Ok(())
    }

    fn audio_track(
        &mut self,
        key: Key,
        path: &Path,
        index: u32,
        timing: &Timing,
    ) -> Result<i32, Error> {
        if let Some(track) = self.cached(key) {
            return Ok(track);
        }
        let track = self
            .builder
            .source(Source::Audio(path.to_path_buf(), index), timing)?;
        self.tracks.push((key, track));
        Ok(track)
    }
}

fn media_path(package: &Path, media: &Media, component: Option<&str>) -> PathBuf {
    let path = package.join(&media.path);
    match component {
        Some(file) if path.is_dir() => path.join(file),
        Some(file) => path.with_file_name(file),
        None => path,
    }
}

fn coordinates(rect: Rect) -> [f64; 4] {
    [rect.x, rect.y, rect.w, rect.h]
}

fn layer(track: i32, destination: [f64; 4], opacity: [f64; 2]) -> VideoLayer {
    VideoLayer {
        track,
        destination,
        opacity,
        crop: [0.0, 0.0, 1.0, 1.0],
        scale: [1.0; 2],
        center_crop: 0,
    }
}

fn ramp(a: f64, b: f64, [from, to]: [Frame; 2]) -> [f64; 2] {
    let steps = to - from - 1;
    [
        a,
        if steps > 0 {
            b + (b - a) / f64::from(steps)
        } else {
            a
        },
    ]
}

fn opacity(a: f64, b: f64, range: [Frame; 2]) -> [f64; 2] {
    ramp(a, b, range).map(|value| value.clamp(0.0, 1.0))
}
