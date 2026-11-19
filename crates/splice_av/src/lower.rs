use crate::{
    Error, TimedText,
    takes::{self, TakeEvents},
};
use apple_sys::composition::{
    AudioRamp, Builder, Composition, CursorImage, CursorSample, Source, TextLayer, Timing,
};
use splice_eval::{
    Animate, Audio, Background, CaptionPreset, Evaluator, Keys, Layer, Picture, TextStyle, Track,
};
use splice_model::{ClipId, Frame, ItemId, Media, MediaId, MediaKind, Project, Rect, TimelineId};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Picture(TimelineId),
    Matte(TimelineId),
    CameraMatte(ClipId),
    Camera(ClipId),
    Audio(TimelineId, Track, bool),
    Fill(usize, u32, u32),
}

/// What a lowered video layer is for, so the GPU can apply the owner's per-frame look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// Drawn as is.
    Plain,
    /// A clip or overlay picture.
    Picture(TimelineId),
    /// The window backing of a picture.
    Backing(TimelineId),
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

/// The lowered layer plan, kept for the OGPU compositor (T12.7).
#[derive(Clone, Default)]
pub(crate) struct Plan {
    /// Instruction intervals in order, each with its layers back to front.
    pub intervals: Vec<([Frame; 2], Vec<Slot>)>,
    /// Cursor tables in registration order (`Slot::cursor` indexes them): samples, images,
    /// points per source pixel, and the take's frame rate.
    pub cursors: Vec<(Vec<CursorSample>, Vec<CursorImage>, f64, f64)>,
    /// Display orientation per video track (0 none … 3 270°).
    pub orients: Vec<(i32, u32)>,
    /// LUT files by `Grade::lut` index.
    pub luts: Vec<PathBuf>,
    /// The prepared timeline, evaluated per frame for keyed looks and shapes.
    pub evaluator: Option<Arc<Evaluator>>,
    /// The black base track under every interval (left out of transparent exports).
    pub base: i32,
}

struct Lowering {
    plan: Plan,
    builder: Builder,
    text: Vec<TimedText>,
    end: Frame,
    tracks: Vec<(Key, i32)>,
    /// Parsed take events and their registered cursor table, per media.
    takes: Vec<(MediaId, Option<TakeEvents>, Option<i32>)>,
}

pub(super) fn build(
    project: &Project,
    package: &Path,
    shared: &Arc<Evaluator>,
    cuts: &[Frame],
    end: Frame,
) -> Result<(Composition, Vec<TimedText>, Plan), Error> {
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
    };
    let black = state.fill(usize::MAX, 0, 0)?;
    state.plan.base = black;
    let [width, height] = project.size.map(f64::from);
    let mut first = evaluator.buffer();
    let mut last = evaluator.buffer();
    let mut chips: Vec<(ClipId, MediaId, Keys)> = Vec::new();
    let mut video = Vec::new();
    let mut tracks = Vec::new();
    let mut audio = Vec::new();
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
                (Layer::Background { fill, .. }, Layer::Background { .. }) => {
                    let [a_color, b_color] = match fill {
                        Background::None => continue,
                        Background::Solid(value) => [value; 2],
                        Background::Gradient(a, b) => [a, b],
                    };
                    let track = state.fill(depth, a_color, b_color)?;
                    // A backing belongs to the picture drawn right after it.
                    let owner = first.layers[depth + 1..].iter().find_map(|l| match l {
                        Layer::Picture(p) => Some(p.item),
                        _ => None,
                    });
                    video.push(Slot::new(
                        track,
                        owner.map_or(Role::Plain, Role::Backing),
                        depth,
                    ));
                }
                (Layer::Fade { color, .. }, Layer::Fade { .. }) => {
                    let track = if color == 0 {
                        black
                    } else {
                        state.fill(depth, color, color)?
                    };
                    video.push(Slot::new(track, Role::Plain, depth));
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
                (Layer::Title { item, bounds, .. }, Layer::Title { .. }) => {
                    if let Some(text) = evaluator.title(item)
                        && !state.text.iter().any(|known| known.item == Some(item))
                    {
                        let end = evaluator
                            .extent(TimelineId::Item(item))
                            .map_or(to, |[_, end]| end);
                        state.title(item, text, bounds, [from, end], evaluator.text_style(item));
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
        tracks.clear();
        tracks.extend(video.iter().map(|slot| slot.track));
        state.builder.range(range, &tracks, &audio)?;
        state.plan.intervals.push((range, video.clone()));
    }
    for (clip, media, style) in chips {
        state.chips(clip, media, style, project, package, evaluator)?;
    }
    Ok((state.builder.finish()?, state.text, state.plan))
}

impl Lowering {
    /// Window corners and shadow, the cursor and click rings, and the click-zoom curve.
    /// The cursor table of a take picture whose cursor is drawn, registered on first use.
    fn cursor_table(
        &mut self,
        picture: Picture,
        project: &Project,
        package: &Path,
    ) -> Result<i32, Error> {
        let media = Self::media(project, picture.media)?;
        if media.kind != MediaKind::Take || picture.cursor.is_none() {
            return Ok(-1);
        }
        let fps = media.probe.fps;
        let Some(index) = self.take(project, package, picture.media)? else {
            return Ok(-1);
        };
        if let Some(table) = self.takes[index].2 {
            return Ok(table);
        }
        match &self.takes[index].1 {
            Some(events) if !events.samples.is_empty() => {
                let table = self.plan.cursors.len() as i32;
                self.plan.cursors.push((
                    events.samples.clone(),
                    events.images.clone(),
                    events.points,
                    fps,
                ));
                self.takes[index].2 = Some(table);
                Ok(table)
            }
            _ => Ok(-1),
        }
    }

    /// Index of the media's parsed take events, loading them on first use.
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
        self.takes.push((id, events, None));
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
        for (n, (t, text)) in keys.iter().enumerate() {
            if hidden.contains(&(n as u32)) {
                continue;
            }
            match bursts.last_mut() {
                Some(last) if t - last.1 < 0.6 && last.2.chars().count() < 24 => {
                    last.1 = *t;
                    last.2.push_str(text);
                }
                _ => bursts.push((*t, *t, text.clone())),
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
        bounds: Rect,
        range: [Frame; 2],
        style: Option<&TextStyle>,
    ) {
        let font = style.map_or(bounds.h * 0.6, |_| bounds.h * 0.6);
        let layer = TextLayer {
            text: text.into(),
            bounds: coordinates(bounds),
            font,
            opacity: [1.0; 2],
            color: style.map_or(0xffffff, |s| s.color),
            style: native_style(style, 0.0),
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
        let units = units(text, style.unit);
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
        let timings = Self::timings(project.fps, evaluator, picture.item, media)?;
        let path = media_path(
            package,
            media,
            media.take.as_ref().and_then(|take| take.screen.as_deref()),
        )?;
        let source = if media.kind == MediaKind::Image {
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
        let path = media_path(package, media, Some(component))?;
        self.insert(key, Source::Video(path), &timings)
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
        let take = media.take.as_ref();
        let (path, index) = match a.track {
            Track::Imported(index) => (media_path(package, media, None)?, index),
            Track::Mic | Track::System => {
                let (name, file) = if a.track == Track::Mic {
                    ("mic", take.and_then(|take| take.mic.as_deref()))
                } else {
                    ("system", take.and_then(|take| take.system.as_deref()))
                };
                if let Some(file) = file {
                    (media_path(package, media, Some(file))?, 0)
                } else {
                    let index = media
                        .probe
                        .audio
                        .iter()
                        .position(|track| track.name == name)
                        .ok_or_else(|| {
                            Error::Input(format!("{} has no {name} audio track.", media.id))
                        })?;
                    (media_path(package, media, None)?, index as u32)
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

/// UTF-16 ranges of the words, characters or lines of `text`.
fn units(text: &str, unit: u8) -> Vec<[usize; 2]> {
    let mut out = Vec::new();
    let mut start = None;
    let mut at = 0;
    for c in text.chars() {
        let boundary = match unit {
            1 => !c.is_whitespace(),
            2 => c != '\n',
            _ => !c.is_whitespace(),
        };
        let width = c.len_utf16();
        if unit == 1 {
            if boundary {
                out.push([at, at + width]);
            }
        } else if boundary {
            start.get_or_insert(at);
        } else if let Some(s) = start.take() {
            out.push([s, at]);
        }
        at += width;
    }
    if let Some(s) = start {
        out.push([s, at]);
    }
    out
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
        },
        None => apple_sys::composition::TextStyle {
            stroke_width: stroke,
            ..Default::default()
        },
    }
}

fn coordinates(rect: Rect) -> [f64; 4] {
    [rect.x, rect.y, rect.w, rect.h]
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
