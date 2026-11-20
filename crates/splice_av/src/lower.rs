use crate::{
    Error, TimedText,
    takes::{self, TakeEvents},
};
use apple_sys::composition::{
    AudioRamp, Builder, Composition, CursorImage, Source, TextLayer, Timing,
};
use splice_eval::cursor;
use splice_eval::{
    Animate, Audio, Background, CaptionPreset, Evaluator, Keys, Layer, Picture, TextStyle, Track,
};
use splice_model::{ClipId, Frame, ItemId, Media, MediaId, MediaKind, Project, TimelineId};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Longest side, in pixels, a Lottie or SVG picture movie is drawn at.
const VECTOR_SIDE: f64 = 1920.0;
/// Frames of a Lottie loop that are drawn (an animated GIF's cap too, in native).
const VECTOR_FRAMES: u32 = 2000;
/// Longest side, in pixels, of an offline media's placeholder picture.
const PLACEHOLDER_SIDE: f64 = 1280.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Picture(TimelineId),
    Matte(TimelineId),
    CameraMatte(ClipId),
    Camera(ClipId),
    Audio(TimelineId, Track, bool),
    Black,
    Backdrop(MediaId, usize),
}

/// What a lowered video layer is for, so the GPU can apply the owner's per-frame look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// Drawn as is.
    Plain,
    /// A clip or overlay picture.
    Picture(TimelineId),
    /// The person matte of a picture: bound to it, never drawn.
    Matte(TimelineId),
    /// A take's camera bubble.
    Camera(ClipId),
    /// The person matte of a camera bubble.
    CameraMatte(ClipId),
}

/// One composited layer of an interval. Its values (bounds, opacity, look, camera, fill) come
/// from the evaluated frame at `depth` every frame; lowering only maps it to a source track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    /// Composition track the decoded source comes from.
    pub track: i32,
    /// What the layer is for.
    pub role: Role,
    /// Index of the evaluated layer it draws, or `NONE` for the black base.
    pub depth: u32,
    /// Cursor table of a take picture, or -1.
    pub cursor: i32,
}

impl Slot {
    /// No evaluated layer (the black base under every interval).
    pub const NONE: u32 = u32::MAX;

    fn new(track: i32, role: Role, depth: usize) -> Self {
        Self {
            track,
            role,
            depth: u32::try_from(depth).unwrap_or(Self::NONE),
            cursor: -1,
        }
    }
}

/// A take's smoothed cursor for one treatment, with its images.
#[derive(Clone)]
pub(crate) struct CursorTable {
    pub media: MediaId,
    pub motion: cursor::Motion,
    pub track: cursor::Track,
    pub images: Vec<CursorImage>,
    /// Captured source width in screen points.
    pub points: f64,
    /// The take's frame rate.
    pub fps: f64,
}

/// The lowered layer plan, kept for the OGPU compositor (T12.7).
#[derive(Clone, Default)]
pub(crate) struct Plan {
    /// Instruction intervals in order, each with its layers back to front.
    pub intervals: Vec<([Frame; 2], Vec<Slot>)>,
    /// Cursor tables in registration order (`Slot::cursor` indexes them).
    pub cursors: Vec<CursorTable>,
    /// Display orientation per video track (0 none … 3 270°).
    pub orients: Vec<(i32, u32)>,
    /// LUT files by `Grade::lut` index.
    pub luts: Vec<PathBuf>,
    /// The prepared timeline, evaluated per frame for keyed looks and shapes.
    pub evaluator: Option<Arc<Evaluator>>,
}

struct Lowering {
    plan: Plan,
    builder: Builder,
    text: Vec<TimedText>,
    end: Frame,
    tracks: Vec<(Key, i32)>,
    /// Parsed take events and their registered cursor table, per media.
    takes: Vec<(MediaId, Option<TakeEvents>)>,
    /// Draw offline media as placeholders instead of failing (renders an agent or the viewer
    /// sees; never export).
    placeholders: bool,
    /// Missing files drawn as placeholders (or left silent, for audio), by media.
    offline: Vec<(MediaId, PathBuf)>,
}

/// What lowering produced: the native composition, its text, the GPU plan and offline files.
pub(super) type Lowered = (Composition, Vec<TimedText>, Plan, Vec<(MediaId, PathBuf)>);

pub(super) fn build(
    project: &Project,
    package: &Path,
    shared: &Arc<Evaluator>,
    cuts: &[Frame],
    end: Frame,
    redact_keys: bool,
    placeholders: bool,
) -> Result<Lowered, Error> {
    let evaluator: &Evaluator = shared;
    let mut luts = Vec::new();
    while let Some(path) = evaluator.lut(luts.len() as u16) {
        luts.push(package.join(path));
    }
    let mut state = Lowering {
        plan: Plan {
            luts,
            evaluator: Some(shared.clone()),
            ..Plan::default()
        },
        builder: Builder::new(project.size, project.fps, end)?,
        text: Vec::new(),
        end,
        tracks: Vec::new(),
        takes: Vec::new(),
        placeholders,
        offline: Vec::new(),
    };
    // One black track stands in for every fill: the GPU draws backgrounds and fades from the
    // evaluated layer, and a fill source decodes to no texture.
    let black = state.black()?;
    let [width, height] = project.size.map(f64::from);
    let mut first = evaluator.buffer();
    let mut last = evaluator.buffer();
    let mut chips: Vec<(ClipId, MediaId, Keys)> = Vec::new();
    let mut video = Vec::new();
    let mut backdrops: Vec<MediaId> = Vec::new();
    let mut tracks = Vec::new();
    let mut audio = Vec::new();
    let clicks = state.click_track(project, package, evaluator)?;
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
        backdrops.clear();
        audio.clear();
        // Keep a real video track under blank, audio-only and text-only intervals.
        if !first
            .layers
            .iter()
            .any(|layer| matches!(layer, Layer::Fade { .. }))
        {
            video.push(Slot::new(black, Role::Plain, usize::MAX));
        }
        for (depth, (&a, &b)) in first.layers.iter().zip(&last.layers).enumerate() {
            match (a, b) {
                (Layer::Picture(a), Layer::Picture(_)) => {
                    if let Some(matte) = state.matte(a, project, package, evaluator)? {
                        video.push(Slot::new(matte, Role::Matte(a.item), depth));
                    }
                    let track = state.picture(a, project, package, evaluator)?;
                    let mut slot = Slot::new(track, Role::Picture(a.item), depth);
                    slot.cursor = state.cursor_table(a, project, package)?;
                    video.push(slot);
                }
                (Layer::Background { fill, .. }, Layer::Background { .. }) => match fill {
                    Background::None => {}
                    // An image backdrop (T46.3) decodes its still; the GPU cover-fits it.
                    Background::Image(media, _) => {
                        // A second use in one frame (an overlay card) gets its own track.
                        let copy = backdrops.iter().filter(|known| **known == media).count();
                        backdrops.push(media);
                        let track = state.backdrop(media, copy, project, package)?;
                        video.push(Slot::new(track, Role::Plain, depth));
                    }
                    _ => video.push(Slot::new(black, Role::Plain, depth)),
                },
                (Layer::Fade { .. }, Layer::Fade { .. }) => {
                    video.push(Slot::new(black, Role::Plain, depth));
                }
                (Layer::Shape { .. }, Layer::Shape { .. }) => {}
                (
                    Layer::Camera {
                        clip,
                        media,
                        background,
                        ..
                    },
                    Layer::Camera { .. },
                ) => {
                    if background != 0 {
                        let matte = state.camera_matte(clip, media, project, package, evaluator)?;
                        video.push(Slot::new(matte, Role::CameraMatte(clip), depth));
                    }
                    let track = state.camera(clip, media, project, package, evaluator)?;
                    video.push(Slot::new(track, Role::Camera(clip), depth));
                }
                (
                    Layer::Title {
                        item,
                        bounds,
                        font,
                        wrap,
                        ..
                    },
                    Layer::Title { .. },
                ) => {
                    let geometry = [bounds.x, bounds.y, bounds.w, bounds.h, font];
                    let runs = evaluator.title_runs(item);
                    // A title is lowered once, with the first interval it is in.
                    let known = state.text.iter().any(|known| known.item == Some(item));
                    if !known && !runs.is_empty() {
                        // A counter (T45.4): one static raster per distinct value, shown over
                        // its run of frames; unit animators do not apply to a changing number.
                        let style = evaluator.text_style(item).map(|style| TextStyle {
                            animate: Animate::None,
                            ..style.clone()
                        });
                        for run in runs {
                            state.title(item, &run.text, geometry, wrap, run.range, style.as_ref());
                        }
                    } else if !known && let Some(text) = evaluator.title(item) {
                        let end = evaluator
                            .extent(TimelineId::Item(item))
                            .map_or(to, |[_, end]| end);
                        state.title(
                            item,
                            text,
                            geometry,
                            wrap,
                            [from, end],
                            evaluator.text_style(item),
                        );
                    }
                }
                (
                    Layer::Caption {
                        item, word, style, ..
                    },
                    Layer::Caption { .. },
                ) => {
                    if let Some(words) = evaluator.caption_words(item) {
                        let count = match style.preset {
                            CaptionPreset::Pop | CaptionPreset::Typewriter => {
                                word.map_or(0, |i| i + 1)
                            }
                            _ => words.len(),
                        };
                        let mut text = String::new();
                        // The spoken word in UTF-16 units, drawn in the active color.
                        let mut active = None;
                        for (index, token) in words[..count].iter().enumerate() {
                            if !text.is_empty() {
                                text.push(' ');
                            }
                            let start = text.encode_utf16().count();
                            text.push_str(&token.text);
                            if word == Some(index) {
                                active = Some([start, text.encode_utf16().count()]);
                            }
                            if let Some(emoji) = &token.emoji {
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
                            let text_style = evaluator.text_style(item);
                            state.add_text(
                                range,
                                Some(item),
                                TextLayer {
                                    text,
                                    bounds: [
                                        width * left,
                                        y,
                                        width * (1.0 - left - right),
                                        content_height,
                                    ],
                                    font,
                                    opacity: [1.0; 2],
                                    color: text_style.map_or(style.color, |s| s.color),
                                    style: native_style(text_style, style.stroke),
                                    visible: None,
                                    active: active
                                        .filter(|_| style.active != style.color)
                                        .map(|range| (range, style.active)),
                                },
                            );
                        }
                    }
                }
                (
                    Layer::Keystrokes {
                        clip, media, style, ..
                    },
                    Layer::Keystrokes { .. },
                ) => {
                    if !chips.iter().any(|(known, ..)| *known == clip) {
                        chips.push((clip, media, style));
                    }
                }
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
        if let Some(track) = clicks {
            audio.push(AudioRamp {
                track,
                gain: [CLICK_GAIN; 2],
            });
        }
        tracks.clear();
        tracks.extend(video.iter().map(|slot| slot.track));
        state.builder.range(range, &tracks, &audio)?;
        state.plan.intervals.push((range, video.clone()));
    }
    // Keystroke chips carry typed characters: renders an agent sees draw the same chips at the
    // same place and time with each character replaced by a dot (D19, A1).
    for (clip, media, style) in chips {
        state.chips(clip, media, style, redact_keys, project, package, evaluator)?;
    }
    Ok((
        state.builder.finish()?,
        state.text,
        state.plan,
        state.offline,
    ))
}

/// Gain of the generated click under the program audio.
const CLICK_GAIN: f32 = 0.8;

impl Lowering {
    /// One audio track with the generated click at every recorded press of take clips whose
    /// cursor effect sets `clickSound` (T46.2), on the timeline where each press plays (through
    /// trims and speed). `None` when nothing clicks.
    fn click_track(
        &mut self,
        project: &Project,
        package: &Path,
        evaluator: &Evaluator,
    ) -> Result<Option<i32>, Error> {
        let mut frames: Vec<Frame> = Vec::new();
        for &slot in project.clips.order() {
            let (Some(id), Some(data)) = (project.clips.id(slot), project.clips.data(slot)) else {
                continue;
            };
            let (true, Some(media), Some(span)) = (
                evaluator.click_sound(id),
                data.media,
                evaluator.source_span(TimelineId::Clip(id)),
            ) else {
                continue;
            };
            let fps = Self::media(project, media)?.probe.fps;
            let marks = Self::media(project, media)?
                .take
                .as_ref()
                .map_or(&[][..], |take| &take.pointer);
            let Some(index) = self.take(project, package, media)? else {
                continue;
            };
            let samples = self.takes[index]
                .1
                .as_ref()
                .map_or(&[][..], |e| e.samples.as_slice());
            let presses = samples
                .iter()
                .filter(|s| s.click)
                .map(|s| s.t)
                .chain(marks.iter().filter(|m| m.click).map(|m| m.t));
            for t in presses {
                let offset = t * fps - span.source;
                let at = match evaluator.speed_map(id) {
                    Some(map) => map.partition_point(|&m| m <= offset) as Frame - 1,
                    None => (offset / span.step).round() as Frame,
                };
                let frame = span.start + at;
                if offset >= 0.0 && (span.start..span.end).contains(&frame) {
                    frames.push(frame);
                }
            }
        }
        if frames.is_empty() {
            return Ok(None);
        }
        frames.sort_unstable();
        frames.dedup();
        let path = crate::audio::click(package)?;
        let fps = f64::from(project.fps);
        let length = (crate::audio::CLICK_SECONDS * fps).ceil().max(1.0) as Frame;
        let mut track = None;
        for (i, &from) in frames.iter().enumerate() {
            // A press soon after another cuts the earlier click short (one track, no overlap).
            let next = frames.get(i + 1).copied().unwrap_or(self.end);
            let to = (from + length).min(next).min(self.end);
            if to <= from {
                continue;
            }
            let timing = Timing {
                timeline: [from, to],
                source_start: 0.0,
                source_limit: crate::audio::CLICK_SECONDS,
                speed: 1.0,
                source_frame_duration: 1.0 / fps,
            };
            let source = Source::Audio(path.clone(), 0);
            match track {
                None => track = Some(self.builder.source(source, &timing)?),
                Some(track) => self.builder.append(track, source, &timing)?,
            }
        }
        Ok(track)
    }

    /// Window corners and shadow, the cursor and click rings, and the click-zoom curve.
    /// The cursor table of a take picture whose cursor is drawn: one smoothed table per take
    /// and cursor treatment (T39), built on first use.
    fn cursor_table(
        &mut self,
        picture: Picture,
        project: &Project,
        package: &Path,
    ) -> Result<i32, Error> {
        let media = Self::media(project, picture.media)?;
        let Some(style) = picture.cursor.filter(|_| media.kind == MediaKind::Take) else {
            return Ok(-1);
        };
        if let Some(table) = self
            .plan
            .cursors
            .iter()
            .position(|c| c.media == picture.media && c.motion == style.motion)
        {
            return Ok(table as i32);
        }
        let fps = media.probe.fps;
        let Some(index) = self.take(project, package, picture.media)? else {
            return Ok(-1);
        };
        match &self.takes[index].1 {
            Some(events) if !events.samples.is_empty() => {
                let images = if style.motion.always_arrow {
                    vec![arrow(package)?]
                } else {
                    events.images.clone()
                };
                self.plan.cursors.push(CursorTable {
                    media: picture.media,
                    motion: style.motion,
                    track: cursor::Track::build(
                        &cursor::with_marks(
                            &events.samples,
                            media.take.as_ref().map_or(&[][..], |take| &take.pointer),
                        ),
                        events.size,
                        style.motion,
                    ),
                    images,
                    points: events.size[0],
                    fps,
                });
                Ok(self.plan.cursors.len() as i32 - 1)
            }
            _ => Ok(-1),
        }
    }

    fn take(
        &mut self,
        project: &Project,
        package: &Path,
        id: MediaId,
    ) -> Result<Option<usize>, Error> {
        if let Some(index) = self.takes.iter().position(|(media, ..)| *media == id) {
            return Ok(self.takes[index].1.is_some().then_some(index));
        }
        let media = Self::media(project, id)?;
        // Events only enrich a take (cursor, zoom, chips): a missing log renders without them.
        let events = match media
            .take
            .as_ref()
            .and_then(|take| take.events.as_deref())
            .and_then(|file| media_path(package, media, Some(file)).ok())
        {
            Some(path) => {
                let cursors = media
                    .take
                    .as_ref()
                    .and_then(|take| take.cursors.as_deref())
                    .and_then(|dir| media_path(package, media, Some(dir)).ok());
                Some(takes::load(&path, cursors.as_deref())?)
            }
            None => None,
        };
        self.takes.push((id, events));
        Ok(self
            .takes
            .last()
            .and_then(|t| t.1.as_ref())
            .map(|_| self.takes.len() - 1))
    }

    /// Keystroke chips as timed text: typing bursts merge into one chip shown for 1.2 s.
    fn chips(
        &mut self,
        clip: ClipId,
        media: MediaId,
        style: Keys,
        redact: bool,
        project: &Project,
        package: &Path,
        evaluator: &Evaluator,
    ) -> Result<(), Error> {
        let Some(index) = self.take(project, package, media)? else {
            return Ok(());
        };
        let Some(span) = evaluator.source_span(TimelineId::Clip(clip)) else {
            return Ok(());
        };
        let fps = Self::media(project, media)?.probe.fps;
        let hidden = evaluator.hidden_keys(clip).unwrap_or(&[]).to_vec();
        let [width, height] = project.size.map(f64::from);
        let chip_h = height * 0.045 * style.scale;
        let y = match style.position {
            splice_eval::KeyPosition::TopLeft => height * 0.08,
            _ => height * 0.86 - chip_h,
        };
        let x = match style.position {
            splice_eval::KeyPosition::BottomCenter => width * 0.2,
            _ => width * 0.06,
        };
        let frame =
            |t: f64| (f64::from(span.start) + (t * fps - span.source) / span.step).round() as Frame;
        let mut bursts: Vec<(f64, f64, String)> = Vec::new();
        let keys = self.takes[index]
            .1
            .as_ref()
            .map_or(&[][..], |e| e.keys.as_slice());
        for (n, (t, text, mods)) in keys.iter().enumerate() {
            let shortcut = mods & splice_eval::activity::SHORTCUT_MODIFIERS != 0;
            if hidden.contains(&(n as u32)) || (style.shortcuts_only && !shortcut) {
                continue;
            }
            let chip = chip_text(text, *mods, redact);
            match bursts.last_mut() {
                Some(last) if t - last.1 < 0.6 && last.2.chars().count() < 24 => {
                    last.1 = *t;
                    last.2.push_str(&chip);
                }
                _ => bursts.push((*t, *t, chip)),
            }
        }
        for (start, end, text) in bursts {
            let from = frame(start).max(span.start);
            let to = frame(end + 1.2).min(span.end);
            if to - from >= 2 {
                self.add_text(
                    [from, to],
                    None,
                    TextLayer {
                        text,
                        bounds: [x, y, width * 0.6, chip_h],
                        font: chip_h * 0.62,
                        opacity: [1.0; 2],
                        color: 0xffffff,
                        style: apple_sys::composition::TextStyle {
                            boxed: 2,
                            box_color: 0x1c1c1e,
                            ..Default::default()
                        },
                        visible: None,
                        active: None,
                    },
                );
            }
        }
        Ok(())
    }

    /// Adds timed text, extending the previous cue when it continues unchanged.
    fn add_text(&mut self, range: [Frame; 2], item: Option<ItemId>, layer: TextLayer) {
        if let Some(last) = self.text.last_mut()
            && item.is_some()
            && last.item == item
            && last.range[1] == range[0]
            && last.layer.text == layer.text
            && last.layer.active == layer.active
            && last.reveal.is_none()
        {
            last.range[1] = range[1];
            return;
        }
        self.text.push(TimedText {
            range,
            layer,
            item,
            reveal: None,
        });
    }

    /// A title: one raster for static text, or one per word/character/line for animators
    /// (each draws only its own range, so the units stay where the full layout puts them).
    fn title(
        &mut self,
        item: ItemId,
        text: &str,
        [x, y, w, h, font]: [f64; 5],
        wrap: bool,
        range: [Frame; 2],
        style: Option<&TextStyle>,
    ) {
        let layer = TextLayer {
            text: text.into(),
            bounds: [x, y, w, h],
            font,
            opacity: [1.0; 2],
            color: style.map_or(0xffffff, |s| s.color),
            style: apple_sys::composition::TextStyle {
                wrap,
                ..native_style(style, 0.0)
            },
            visible: None,
            active: None,
        };
        let Some(style) = style.filter(|s| s.animate != Animate::None) else {
            self.text.push(TimedText {
                range,
                layer,
                item: Some(item),
                reveal: None,
            });
            return;
        };
        if style.animate == Animate::Highlight {
            // The full text stays; each unit lights up over it in the accent color.
            self.text.push(TimedText {
                range,
                layer: layer.clone(),
                item: Some(item),
                reveal: None,
            });
        }
        // Characters are grapheme clusters, so emoji sequences animate whole (T42.3).
        let units = splice_eval::text_units(text, style.unit);
        for (index, unit) in units.into_iter().enumerate() {
            let start = range[0] + (index as f64 * style.stagger).round() as Frame;
            let mut part = layer.clone();
            part.visible = Some(unit);
            if style.animate == Animate::Highlight {
                part.color = 0xffd23f;
            }
            self.text.push(TimedText {
                range: [start.min(range[1] - 1), range[1]],
                layer: part,
                item: Some(item),
                reveal: Some((start, style.duration, style.animate)),
            });
        }
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

    /// Track segments for an item: one linear range over its extent (transition overlap
    /// included), or 2-frame pieces following a speed ramp; a held frame is one source frame
    /// stretched over its piece.
    fn timings(
        fps: u32,
        evaluator: &Evaluator,
        item: TimelineId,
        media: &Media,
    ) -> Result<Vec<Timing>, Error> {
        let span = evaluator
            .source_span(item)
            .ok_or_else(|| Error::Input(format!("Missing source timing for {item}.")))?;
        let source_fps = media.probe.fps;
        let limit = f64::from(media.probe.frames) / source_fps;
        let frame = 1.0 / source_fps;
        let timing = |from: Frame, to: Frame, source: f64, step: f64| {
            let seconds = f64::from(to - from) / f64::from(fps);
            let speed = if step * f64::from(to - from) < 0.5 {
                frame / seconds
            } else {
                step * f64::from(fps) / source_fps
            };
            Timing {
                timeline: [from, to],
                source_start: (source / source_fps).max(0.0),
                source_limit: limit,
                speed,
                source_frame_duration: frame,
            }
        };
        if let TimelineId::Clip(id) = item
            && let Some(map) = evaluator.speed_map(id)
        {
            let mut out = Vec::new();
            let mut at = span.start;
            while at < span.end {
                let next = (at + 2).min(span.end);
                let a = map[(at - span.start) as usize];
                let b = map[(next - span.start) as usize];
                out.push(timing(
                    at,
                    next,
                    span.source + a,
                    (b - a) / f64::from(next - at),
                ));
                at = next;
            }
            return Ok(out);
        }
        let [mut from, to] = evaluator.extent(item).unwrap_or([span.start, span.end]);
        let mut source = span.source - f64::from(span.start - from) * span.step;
        let mut out = Vec::with_capacity(2);
        if source < 0.0 {
            // No head handle: a still has every frame; a video holds its first frame.
            if media.kind == MediaKind::Image {
                source = 0.0;
            } else {
                let skip = ((-source / span.step).ceil() as Frame).min(to - from);
                out.push(timing(from, from + skip, 0.0, 0.0));
                from += skip;
                source += f64::from(skip) * span.step;
            }
        }
        if from < to {
            out.push(timing(from, to, source, span.step));
        }
        Ok(out)
    }

    /// Adds `source` as a track over `timings`, returning the track.
    fn insert(&mut self, key: Key, source: Source, timings: &[Timing]) -> Result<i32, Error> {
        let (first, rest) = timings
            .split_first()
            .ok_or_else(|| Error::Input("An item has no frames to render.".into()))?;
        let track = self.builder.source(source.clone(), first)?;
        for timing in rest {
            self.builder.append(track, source.clone(), timing)?;
        }
        self.tracks.push((key, track));
        Ok(track)
    }

    /// The person matte track for a picture with a cutout, from the analysis store.
    fn matte(
        &mut self,
        picture: Picture,
        project: &Project,
        package: &Path,
        evaluator: &Evaluator,
    ) -> Result<Option<i32>, Error> {
        if picture.look.cutout.is_none() {
            return Ok(None);
        }
        let key = Key::Matte(picture.item);
        if let Some(track) = self.cached(key) {
            return Ok(Some(track));
        }
        let media = Self::media(project, picture.media)?;
        let path = crate::matte_path(package, media.id);
        if !path.exists() {
            return Err(Error::Input(format!(
                "{} has no person matte. Run media.analyze {{media:\"{}\", kinds:[\"people\"]}} first.",
                media.id, media.id
            )));
        }
        let timings = Self::timings(project.fps, evaluator, picture.item, media)?;
        self.insert(key, Source::Video(path), &timings).map(Some)
    }

    fn black(&mut self) -> Result<i32, Error> {
        let key = Key::Black;
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
        let track = self.builder.source(Source::Color(0, 0), &timing)?;
        self.tracks.push((key, track));
        Ok(track)
    }

    /// A window's image backdrop (T46.3): one still track per image over the whole timeline,
    /// shared by every clip that uses it.
    fn backdrop(
        &mut self,
        id: MediaId,
        copy: usize,
        project: &Project,
        package: &Path,
    ) -> Result<i32, Error> {
        let key = Key::Backdrop(id, copy);
        if let Some(track) = self.cached(key) {
            return Ok(track);
        }
        let media = Self::media(project, id)?;
        let (path, placeholder) = self.picture_path(package, media, None)?;
        if !placeholder && matches!(media.probe.codec.as_deref(), Some("lottie" | "svg")) {
            self.vector(&path, project.size)?;
        }
        let timing = Timing {
            timeline: [0, self.end],
            source_start: 0.0,
            source_limit: 1.0,
            speed: 1.0,
            source_frame_duration: 1.0,
        };
        let track = self.builder.source(Source::Still(path), &timing)?;
        self.tracks.push((key, track));
        let orient = self.builder.orientation(track);
        if orient != 0 {
            self.plan.orients.push((track, orient));
        }
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
        let timings = Self::timings(project.fps, evaluator, picture.item, media)?;
        let (path, placeholder) = self.picture_path(
            package,
            media,
            media.take.as_ref().and_then(|take| take.screen.as_deref()),
        )?;
        let source = if placeholder {
            Source::Still(path)
        } else if media.kind == MediaKind::Image {
            if matches!(media.probe.codec.as_deref(), Some("lottie" | "svg")) {
                self.vector(&path, project.size)?;
            }
            Source::Still(path)
        } else {
            Source::Video(path)
        };
        let track = self.insert(key, source, &timings)?;
        let orient = self.builder.orientation(track);
        if orient != 0 {
            self.plan.orients.push((track, orient));
        }
        Ok(track)
    }

    /// Draws a Lottie or SVG file (T45.2) into the picture movie its still source plays: every
    /// frame of a Lottie at its own rate (the movie loops across the item, like an animated
    /// GIF), one frame for an SVG. Transparent where the file draws nothing.
    fn vector(&mut self, path: &Path, canvas: [u32; 2]) -> Result<(), Error> {
        let unreadable = |failure: thorvg_sys::Error| {
            Error::Input(format!("{} could not be drawn: {failure}", path.display()))
        };
        let mut animation = thorvg_sys::open(path).map_err(unreadable)?;
        let info = animation.info();
        // ponytail: drawn to fit the canvas, at most 1920 px on its longest side; draw at the
        // item's on-screen size if zoomed vectors look soft.
        let [w, h] = [f64::from(info.width), f64::from(info.height)];
        let fit = (f64::from(canvas[0]) / w)
            .min(f64::from(canvas[1]) / h)
            .min(VECTOR_SIDE / w.max(h));
        // Even and at least 32 px, as still_buffer encodes it.
        let side = |value: f64| ((value * fit).round().clamp(32.0, VECTOR_SIDE) as u32 + 1) & !1;
        let size = [side(w), side(h)];
        let (count, seconds) = if info.frames > 1 {
            (info.frames.min(VECTOR_FRAMES) as usize, 1.0 / info.fps)
        } else {
            (1, 1.0 / 30.0)
        };
        let mut failure = None;
        let made =
            self.builder.frames(
                path,
                size,
                count,
                seconds,
                &mut |index, rgba| match animation.render(index as u32, size, rgba) {
                    Ok(()) => true,
                    Err(error) => {
                        failure = Some(error);
                        false
                    }
                },
            );
        match failure {
            Some(error) => Err(unreadable(error)),
            None => made.map_err(Error::from),
        }
    }

    /// The camera component's person matte (`media.analyze {kinds:["matte"]}`).
    fn camera_matte(
        &mut self,
        clip: ClipId,
        id: MediaId,
        project: &Project,
        package: &Path,
        evaluator: &Evaluator,
    ) -> Result<i32, Error> {
        let key = Key::CameraMatte(clip);
        if let Some(track) = self.cached(key) {
            return Ok(track);
        }
        let path = crate::camera_matte_path(package, id);
        if !path.exists() {
            return Err(Error::Input(format!(
                "{id} has no camera matte. Run media.analyze {{kinds:[\"matte\"]}} on the take first."
            )));
        }
        let media = Self::media(project, id)?;
        let timings = Self::timings(project.fps, evaluator, TimelineId::Clip(clip), media)?;
        self.insert(key, Source::Video(path), &timings)
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
        let timings = Self::timings(project.fps, evaluator, TimelineId::Clip(clip), media)?;
        let component = media
            .take
            .as_ref()
            .and_then(|take| take.camera.as_deref())
            .ok_or_else(|| Error::Input(format!("{id} has no camera component.")))?;
        let (path, placeholder) = self.picture_path(package, media, Some(component))?;
        let source = if placeholder {
            Source::Still(path)
        } else {
            Source::Video(path)
        };
        self.insert(key, source, &timings)
    }

    /// The file a picture plays, or (`true`) a drawn placeholder still under the missing path
    /// when the file is offline and this render allows placeholders.
    fn picture_path(
        &mut self,
        package: &Path,
        media: &Media,
        component: Option<&str>,
    ) -> Result<(PathBuf, bool), Error> {
        match media_path(package, media, component) {
            Err(Error::Offline(path)) if self.placeholders => {
                if !self.offline.iter().any(|(_, known)| *known == path) {
                    self.placeholder(media, &path)?;
                    self.offline.push((media.id, path.clone()));
                }
                Ok((path, true))
            }
            other => other.map(|path| (path, false)),
        }
    }

    /// Draws the placeholder picture movie for a missing file at the media's aspect: a dark
    /// fill, a diagonal hatch and "Media offline: <file name>", so the item's window, crop and
    /// transforms stay checkable.
    fn placeholder(&mut self, media: &Media, path: &Path) -> Result<(), Error> {
        let [w, h] = match [media.probe.width, media.probe.height] {
            [0, _] | [_, 0] => [1280.0, 720.0],
            [w, h] => [f64::from(w), f64::from(h)],
        };
        let fit = PLACEHOLDER_SIDE / w.max(h);
        // Even and at least 32 px, as still_buffer encodes it.
        let side =
            |value: f64| ((value * fit).round().clamp(32.0, PLACEHOLDER_SIDE) as u32 + 1) & !1;
        let size = [side(w), side(h)];
        let name = path
            .file_name()
            .map_or_else(|| media.path.clone(), |n| n.to_string_lossy().into_owned());
        let font = (f64::from(size[0].min(size[1])) / 10.0).max(8.0);
        let band = [size[0] * 9 / 10, ((font * 2.5).round() as u32).min(size[1])];
        let layer = TextLayer {
            text: format!("Media offline: {name}"),
            bounds: [0.0, 0.0, f64::from(band[0]), f64::from(band[1])],
            font,
            opacity: [1.0; 2],
            color: 0xe0e0e4,
            style: apple_sys::composition::TextStyle {
                wrap: false,
                ..Default::default()
            },
            visible: None,
            active: None,
        };
        let text = apple_sys::composition::text_rgba(&layer, band)?;
        let origin = [(size[0] - band[0]) / 2, (size[1] - band[1]) / 2];
        let stripe = (size[0].max(size[1]) / 24).max(8);
        let mut fill = |_: usize, rgba: &mut [u8]| {
            for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                let [x, y] = [index as u32 % size[0], index as u32 / size[0]];
                let base: [f32; 3] = if (x + y) % stripe < stripe / 4 {
                    [60.0, 60.0, 66.0]
                } else {
                    [38.0, 38.0, 42.0]
                };
                let [tx, ty] = [x.wrapping_sub(origin[0]), y.wrapping_sub(origin[1])];
                let ink = if tx < band[0] && ty < band[1] {
                    let at = (ty * band[0] + tx) as usize * 4;
                    Some((&text[at..at + 3], f32::from(text[at + 3]) / 255.0))
                } else {
                    None
                };
                for channel in 0..3 {
                    pixel[channel] = match ink {
                        Some((color, alpha)) => {
                            base[channel] * (1.0 - alpha) + f32::from(color[channel]) * alpha
                        }
                        None => base[channel],
                    } as u8;
                }
                pixel[3] = 255;
            }
            true
        };
        self.builder
            .frames(path, size, 1, 1.0 / 30.0, &mut fill)
            .map_err(Error::from)
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
        let timings = Self::timings(project.fps, evaluator, a.item, media)?;
        let (path, index) = match audio_source(package, media, a.track) {
            // Renders that allow placeholders have no sound: offline audio is only reported.
            Err(Error::Offline(path)) if self.placeholders => {
                if !self.offline.iter().any(|(_, known)| *known == path) {
                    self.offline.push((media.id, path));
                }
                return Ok(());
            }
            other => other?,
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
        let (path, index) = crate::audio::processed(package, evaluator, a, &path, index)?;
        let track = self.audio_track(Key::Audio(a.item, a.track, false), &path, index, &timings)?;
        out.push(AudioRamp { track, gain });
        // AVFoundation supports up to unity per input. A second copy contributes the surplus.
        if peak > 1.0 {
            let track =
                self.audio_track(Key::Audio(a.item, a.track, true), &path, index, &timings)?;
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
        timings: &[Timing],
    ) -> Result<i32, Error> {
        if let Some(track) = self.cached(key) {
            return Ok(track);
        }
        self.insert(key, Source::Audio(path.to_path_buf(), index), timings)
    }
}

/// The recorded pointers of the project's takes, for zooms that follow the cursor (T46.2):
/// empty when no zoom follows. A take whose event log is missing or offline is left out (its
/// following zooms hold their framing).
pub(super) fn pointers(project: &Project, package: &Path) -> Result<Vec<cursor::Recorded>, Error> {
    let follows = project.items.order().iter().any(|&slot| {
        matches!(
            project.items.data(slot).map(|d| &d.kind),
            Some(splice_model::ItemKind::Zoom(zoom)) if zoom.follow
        )
    });
    let mut out = Vec::new();
    for media in project
        .media
        .iter()
        .filter(|m| follows && m.kind == MediaKind::Take)
    {
        // ponytail: lowering parses the log again for the drawn cursor; share the parse if
        // composition builds of long takes with following zooms show it.
        let Some(path) = media
            .take
            .as_ref()
            .and_then(|take| take.events.as_deref())
            .and_then(|file| media_path(package, media, Some(file)).ok())
        else {
            continue;
        };
        // With the cursor images, so image changes land in the samples exactly as they do for
        // the drawn table.
        let cursors = media
            .take
            .as_ref()
            .and_then(|take| take.cursors.as_deref())
            .and_then(|dir| media_path(package, media, Some(dir)).ok());
        let events = takes::load(&path, cursors.as_deref())?;
        out.push(cursor::Recorded {
            media: media.id,
            size: events.size,
            samples: events.samples,
        });
    }
    Ok(out)
}

/// The always-arrow image (the cursor effect's `alwaysArrow`), generated by splice_eval and
/// written once into the package cache so the GPU loads it like a recorded cursor image.
fn arrow(package: &Path) -> Result<CursorImage, Error> {
    const SCALE: u32 = 2;
    let dir = package.join("cache").join("cursors");
    let path = dir.join("arrow-v1.png");
    let (rgba, size, hot) = cursor::arrow(SCALE);
    if !path.exists() {
        let failed = |e: std::io::Error| Error::Input(format!("{}: {e}", dir.display()));
        std::fs::create_dir_all(&dir).map_err(failed)?;
        let partial = dir.join("arrow-v1.partial.png");
        std::fs::write(&partial, apple_sys::image::png(&rgba, size, 0)?).map_err(failed)?;
        std::fs::rename(&partial, &path).map_err(failed)?;
    }
    Ok(CursorImage {
        path,
        hot,
        scale: f64::from(SCALE),
    })
}

/// The file and audio track index a media's audio track plays from.
fn audio_source(package: &Path, media: &Media, track: Track) -> Result<(PathBuf, u32), Error> {
    let take = media.take.as_ref();
    match track {
        Track::Imported(index) => Ok((media_path(package, media, None)?, index)),
        Track::Mic | Track::System => {
            let (name, file) = if track == Track::Mic {
                ("mic", take.and_then(|take| take.mic.as_deref()))
            } else {
                ("system", take.and_then(|take| take.system.as_deref()))
            };
            if let Some(file) = file {
                return Ok((media_path(package, media, Some(file))?, 0));
            }
            let index = media
                .probe
                .audio
                .iter()
                .position(|track| track.name == name)
                .ok_or_else(|| Error::Input(format!("{} has no {name} audio track.", media.id)))?;
            Ok((media_path(package, media, None)?, index as u32))
        }
    }
}

fn media_path(package: &Path, media: &Media, component: Option<&str>) -> Result<PathBuf, Error> {
    let path = package.join(&media.path);
    let path = match component {
        Some(file) if path.is_dir() => path.join(file),
        Some(file) => path.with_file_name(file),
        None => path,
    };
    if path.exists() {
        Ok(path)
    } else {
        Err(Error::Offline(path))
    }
}

/// The native raster style for an eval text style (captions pass their stroke).
fn native_style(style: Option<&TextStyle>, stroke: f64) -> apple_sys::composition::TextStyle {
    match style {
        Some(s) => apple_sys::composition::TextStyle {
            font: s.font.clone(),
            weight: u32::from(s.weight),
            stroke: s.stroke,
            stroke_width: s.stroke_width.max(stroke),
            shadow: s.shadow,
            boxed: u32::from(s.boxed),
            box_color: s.box_color,
            align: u32::from(s.align),
            line_height: s.line_height,
            wrap: true,
        },
        None => apple_sys::composition::TextStyle {
            stroke_width: stroke,
            ..Default::default()
        },
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

/// One keystroke's chip text: a shortcut carries its modifier glyphs (⌃⌥⇧⌘, Apple's order) and
/// an upper-case key. Redacted (renders an agent sees, D19) every typed character is a dot, so
/// the chip keeps its place, timing and rough width without the text.
fn chip_text(text: &str, mods: u64, redact: bool) -> String {
    let mut chip = String::new();
    if mods & splice_eval::activity::SHORTCUT_MODIFIERS != 0 {
        for (bit, glyph) in [(18, '⌃'), (19, '⌥'), (17, '⇧'), (20, '⌘')] {
            if mods & (1 << bit) != 0 {
                chip.push(glyph);
            }
        }
    }
    if redact {
        chip.extend(text.chars().map(|_| '•'));
    } else if chip.is_empty() {
        chip.push_str(text);
    } else {
        chip.extend(text.chars().flat_map(char::to_uppercase));
    }
    chip
}

#[cfg(test)]
mod chip_tests {
    use super::chip_text;

    #[test]
    fn chips_show_shortcut_glyphs_and_agent_renders_redact_the_text() {
        let command = 1 << 20;
        let shift = 1 << 17;
        assert_eq!(chip_text("s", command, false), "⌘S");
        assert_eq!(chip_text("s", command | shift | (1 << 18), false), "⌃⇧⌘S");
        assert_eq!(chip_text("s", command, true), "⌘•");
        // Shift alone is typing: the text as typed, or dots for an agent.
        assert_eq!(chip_text("P", shift, false), "P");
        assert_eq!(chip_text("pw", 0, true), "••");
    }
}
