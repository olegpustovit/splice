use super::{
    Audio, Background, CaptionStyle, CaptionWord, FramePlan, Keys, Layer, Picture, Track, layout,
    style,
};
use splice_model::timing::{self, TimingError};
use splice_model::*;
use std::fmt;

/// A project lacks valid geometry or timeline references for playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalError {
    /// Invalid timeline anchor or source clock.
    Timing(TimingError),
    /// Canvas size or frame rate is zero.
    Canvas,
    /// Invalid range or overlapping spine clips.
    Range,
    /// An overlay has no matching lane.
    Lane(LaneId),
    /// A media descriptor is unavailable.
    Media(MediaId),
    /// Video dimensions or crop are invalid.
    Geometry(MediaId),
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cannot prepare render plan: {self:?}")
    }
}

impl std::error::Error for EvalError {}

impl From<TimingError> for EvalError {
    fn from(error: TimingError) -> Self {
        Self::Timing(error)
    }
}

/// Prepared playback state for one immutable project snapshot.
pub struct Evaluator {
    size: [u32; 2],
    clips: Vec<PreparedClip>,
    overlays: Vec<PreparedOverlay>,
    titles: Vec<PreparedTitle>,
    captions: Vec<PreparedCaption>,
    motions: Vec<PreparedMotion>,
}

struct PreparedClip {
    id: ClipId,
    interval: Interval,
    input: Option<Input>,
    take: bool,
    visual: Option<Visual>,
    camera: Option<Camera>,
    keys: Option<Keys>,
    hidden_keys: Vec<u32>,
}

struct Input {
    id: MediaId,
    source: f64,
    step: f64,
    frames: f64,
    tracks: Vec<(Track, f64)>,
}

impl Input {
    fn frame(&self, offset: Frame) -> f64 {
        self.source + f64::from(offset) * self.step
    }

    fn audio(&self, at: f64, item: TimelineId, fade: f64, out: &mut FramePlan) {
        if at >= 0.0 && at < self.frames {
            out.audio
                .extend(self.tracks.iter().map(|&(track, gain)| Audio {
                    item,
                    media: self.id,
                    track,
                    source_frame: at,
                    gain: fade * gain,
                }));
        }
    }
}

struct Camera {
    bounds: Rect,
    radius: f64,
    shadow: f64,
}

struct Visual {
    bounds: Rect,
    visible: bool,
    picture: Picture,
}

impl Visual {
    fn draw(&self, at: f64, scale: f64, fade: f64, out: &mut FramePlan) {
        if let Some(window) = self.picture.window
            && window.background != Background::None
        {
            out.layers.push(Layer::Background {
                bounds: self.bounds,
                fill: window.background,
                opacity: fade,
            });
        }
        if self.visible {
            out.layers.push(Layer::Picture(Picture {
                source_frame: at,
                scale,
                opacity: fade,
                ..self.picture
            }));
        }
    }
}

struct PreparedOverlay {
    interval: Interval,
    input: Input,
    visual: Visual,
    behind: bool,
}

struct PreparedTitle {
    id: ItemId,
    interval: Interval,
    text: String,
}

struct PreparedMotion {
    interval: Interval,
    from: f64,
    to: f64,
}

struct PreparedCaption {
    id: ItemId,
    interval: Interval,
    style: CaptionStyle,
    words: Vec<CaptionWord>,
}

#[derive(Clone, Copy)]
struct Interval {
    start: Frame,
    end: Frame,
    fade_in: Frame,
    fade_out: Frame,
}

impl Interval {
    fn new(start: Frame, length: Frame, appearance: &Appearance) -> Result<Self, EvalError> {
        let end = start.checked_add(length).ok_or(EvalError::Range)?;
        if start < 0 || length <= 0 {
            return Err(EvalError::Range);
        }
        Ok(Self {
            start,
            end,
            fade_in: appearance.fade_in,
            fade_out: appearance.fade_out,
        })
    }

    fn contains(self, at: Frame) -> bool {
        at >= self.start && at < self.end
    }

    fn alpha(self, at: Frame) -> f64 {
        let mut alpha: f64 = 1.0;
        if self.fade_in > 0 {
            alpha = alpha.min(f64::from(at - self.start + 1) / f64::from(self.fade_in));
        }
        if self.fade_out > 0 {
            alpha = alpha.min(f64::from(self.end - at) / f64::from(self.fade_out));
        }
        alpha.clamp(0.0, 1.0)
    }
}

impl Evaluator {
    /// Resolves anchors, source clocks, geometry, and effects. Call again after a project edit.
    pub fn prepare(project: &Project) -> Result<Self, EvalError> {
        if project.fps == 0 || project.size.contains(&0) {
            return Err(EvalError::Canvas);
        }
        let size = project.size.map(f64::from);
        let mut result = Self {
            size: project.size,
            clips: Vec::with_capacity(project.clips.order().len()),
            overlays: Vec::new(),
            titles: Vec::new(),
            captions: Vec::new(),
            motions: Vec::new(),
        };
        let mut end = 0;
        for &slot in project.clips.order() {
            let clip = prepare_clip(project, slot, size)?;
            if clip.interval.start < end {
                return Err(EvalError::Range);
            }
            end = clip.interval.end;
            result.clips.push(clip);
        }
        let mut ordered = Vec::with_capacity(project.items.order().len());
        for &slot in project.items.order() {
            let item = project.items.data(slot).ok_or(EvalError::Range)?;
            let rank = project
                .lanes
                .iter()
                .position(|lane| lane.id == item.lane && lane.kind == item.kind.lane_kind())
                .ok_or(EvalError::Lane(item.lane))?;
            ordered.push((rank, slot));
        }
        ordered.sort_by_key(|&(rank, _)| std::cmp::Reverse(rank));
        for (rank, slot) in ordered {
            result.prepare_item(project, rank, slot, size)?;
        }
        // Prefer motion from the highest active lane.
        result.motions.reverse();
        Ok(result)
    }

    /// Reserves enough space to evaluate any frame of this snapshot.
    pub fn buffer(&self) -> FramePlan {
        let clip_tracks = self
            .clips
            .iter()
            .filter_map(|clip| clip.input.as_ref())
            .map(|input| input.tracks.len())
            .max()
            .unwrap_or(0);
        let overlay_tracks = self
            .overlays
            .iter()
            .map(|overlay| overlay.input.tracks.len())
            .sum::<usize>();
        FramePlan {
            layers: Vec::with_capacity(
                5 + self.overlays.len() * 2 + self.titles.len() + self.captions.len(),
            ),
            audio: Vec::with_capacity(clip_tracks + overlay_tracks),
            ..FramePlan::default()
        }
    }

    /// Fills a reusable render plan. A buffer from this evaluator allocates nothing here.
    pub fn frame(&self, at: Frame, out: &mut FramePlan) {
        out.frame = at;
        out.size = self.size;
        out.layers.clear();
        out.audio.clear();
        let index = self.clips.partition_point(|clip| clip.interval.start <= at);
        let clip = index
            .checked_sub(1)
            .and_then(|i| self.clips.get(i))
            .filter(|clip| clip.interval.contains(at));
        out.clip = clip.map(|clip| clip.id);
        if let Some(clip) = clip {
            self.draw_clip(clip, at, out);
        }
        self.draw_overlays(at, clip.is_some_and(|clip| clip.take), false, out);
        let [w, h] = self.size.map(f64::from);
        let height = h.min(w) * 0.1;
        for (index, title) in self
            .titles
            .iter()
            .filter(|title| title.interval.contains(at))
            .enumerate()
        {
            out.layers.push(Layer::Title {
                item: title.id,
                bounds: layout::rect(
                    w * 0.06,
                    h * 0.07 + index as f64 * height * 1.2,
                    w * 0.88,
                    height,
                ),
                opacity: title.interval.alpha(at),
            });
        }
        for caption in self
            .captions
            .iter()
            .filter(|caption| caption.interval.contains(at))
        {
            let word = caption.words.iter().rposition(|word| at >= word.start);
            out.layers.push(Layer::Caption {
                item: caption.id,
                word,
                opacity: caption.interval.alpha(at),
                style: caption.style,
            });
        }
    }

    /// Prepared title text.
    pub fn title(&self, id: ItemId) -> Option<&str> {
        self.titles
            .iter()
            .find(|title| title.id == id)
            .map(|title| title.text.as_str())
    }

    /// Prepared text and timing for the words in a caption cue.
    pub fn caption_words(&self, id: ItemId) -> Option<&[CaptionWord]> {
        self.captions
            .iter()
            .find(|caption| caption.id == id)
            .map(|caption| caption.words.as_slice())
    }

    /// Keystroke event indices hidden on a source clip.
    pub fn hidden_keys(&self, id: ClipId) -> Option<&[u32]> {
        self.clips
            .iter()
            .find(|clip| clip.id == id)
            .map(|clip| clip.hidden_keys.as_slice())
    }

    fn draw_clip(&self, clip: &PreparedClip, frame: Frame, out: &mut FramePlan) {
        let Some(input) = &clip.input else { return };
        let source = input.frame(frame - clip.interval.start);
        let opacity = clip.interval.alpha(frame);
        if let Some(visual) = &clip.visual {
            let zoom = self
                .motions
                .iter()
                .find(|motion| motion.interval.contains(frame))
                .map_or(1.0, |motion| {
                    let progress = f64::from(frame - motion.interval.start)
                        / f64::from((motion.interval.end - motion.interval.start - 1).max(1));
                    motion.from + (motion.to - motion.from) * progress
                });
            visual.draw(source, zoom, 1.0, out);
        }
        input.audio(source, TimelineId::Clip(clip.id), opacity, out);
        if clip.take {
            self.draw_overlays(frame, true, true, out);
        }
        if let Some(camera) = &clip.camera {
            out.layers.push(Layer::Camera {
                clip: clip.id,
                media: input.id,
                source_frame: source,
                bounds: camera.bounds,
                radius: camera.radius,
                shadow: camera.shadow,
            });
        }
        if let Some(style) = clip.keys {
            out.layers.push(Layer::Keystrokes {
                clip: clip.id,
                media: input.id,
                source_frame: source,
                style,
            });
        }
        if opacity < 1.0 {
            out.layers.push(Layer::Fade {
                opacity: 1.0 - opacity,
            });
        }
    }

    fn draw_overlays(&self, frame: Frame, on_take: bool, behind: bool, out: &mut FramePlan) {
        for overlay in self.overlays.iter().filter(|overlay| {
            (overlay.behind && on_take) == behind && overlay.interval.contains(frame)
        }) {
            let opacity = overlay.interval.alpha(frame);
            let source = overlay.input.frame(frame - overlay.interval.start);
            overlay.visual.draw(
                source.min((overlay.input.frames - 1.0).max(0.0)),
                1.0,
                opacity,
                out,
            );
            overlay
                .input
                .audio(source, overlay.visual.picture.item, opacity, out);
        }
    }

    fn prepare_item(
        &mut self,
        p: &Project,
        rank: usize,
        slot: Slot,
        size: [f64; 2],
    ) -> Result<(), EvalError> {
        let id = p.items.id(slot).ok_or(EvalError::Range)?;
        let data = p.items.data(slot).ok_or(EvalError::Range)?;
        let timing = p.items.timing(slot).ok_or(EvalError::Range)?;
        let start = timing::anchor_start(p, timing.anchor)?;
        let interval = Interval::new(start, timing.length, &data.appearance)?;
        let lane = &p.lanes[rank].effects;
        let local = &data.appearance.effects;
        match &data.kind {
            ItemKind::Title { text } => self.titles.push(PreparedTitle {
                id,
                interval,
                text: text.clone(),
            }),
            ItemKind::Motion { from, to } => {
                if !from.is_finite() || !to.is_finite() || *from <= 0.0 || *to <= 0.0 {
                    return Err(EvalError::Range);
                }
                self.motions.push(PreparedMotion {
                    interval,
                    from: *from,
                    to: *to,
                });
            }
            ItemKind::Media(overlay) => {
                let media = media(p, overlay.media)?;
                let window = style::window(&[lane, local]);
                let tracks = if overlay.muted {
                    Vec::new()
                } else {
                    overlay_tracks(media)
                };
                let input = input(
                    media,
                    overlay.source,
                    media.probe.fps / f64::from(p.fps),
                    tracks,
                )?;
                let bounds = layout::overlay(overlay.placement, size);
                let visual = if media.kind == MediaKind::Audio {
                    blank_visual(media.id, TimelineId::Item(id), bounds, None)
                } else {
                    visual(media, TimelineId::Item(id), bounds, overlay.framing, window)?
                };
                self.overlays.push(PreparedOverlay {
                    interval,
                    input,
                    visual,
                    behind: overlay.behind_presenter,
                });
            }
            ItemKind::Caption { words, edits } => {
                if let Some(style) = style::caption(&[&p.effects, lane, local]) {
                    let words = caption_tokens(p, timing.anchor, words, edits, style.uppercase)?;
                    if !words.is_empty() {
                        self.captions.push(PreparedCaption {
                            id,
                            interval,
                            style,
                            words,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

fn prepare_clip(p: &Project, slot: Slot, size: [f64; 2]) -> Result<PreparedClip, EvalError> {
    let id = p.clips.id(slot).ok_or(EvalError::Range)?;
    let data = p.clips.data(slot).ok_or(EvalError::Range)?;
    let timing = p.clips.timing(slot).ok_or(EvalError::Range)?;
    let interval = Interval::new(timing.start, timing.length, &data.appearance)?;
    let mut clip = PreparedClip {
        id,
        interval,
        input: None,
        take: false,
        visual: None,
        camera: None,
        keys: None,
        hidden_keys: data.hidden_keys.clone(),
    };
    let Some(media_id) = data.media else {
        return Ok(clip);
    };
    let media = media(p, media_id)?;
    let scopes: [&[Effect]; 3] = [&p.effects, &p.spine.effects, &data.appearance.effects];
    clip.take = media.kind == MediaKind::Take;
    let enabled = &data.enabled;
    let screen = !clip.take || enabled.screen == Some(true);
    let tracks = if clip.take {
        take_tracks(enabled, data.volume)
    } else {
        file_tracks(media)
    };
    let input = input(media, timing.source, timing::source_rate(p, slot)?, tracks)?;
    if input.frame(timing.length) > input.frames + 0.000_001 {
        return Err(EvalError::Range);
    }
    clip.input = Some(input);
    if media.kind != MediaKind::Audio {
        let [w, h] = size;
        let window = style::window(&scopes);
        let bounds = layout::rect(0.0, 0.0, w, h);
        let mut visual = if screen {
            visual(media, TimelineId::Clip(id), bounds, data.framing, window)?
        } else {
            // A camera-only take still has a backdrop even without a screen size.
            blank_visual(media.id, TimelineId::Clip(id), bounds, window)
        };
        if clip.take {
            if enabled.pointer == Some(true) {
                visual.picture.cursor = style::cursor(&scopes);
            }
            // ponytail: pass click settings and source clock through the plan until
            // T8 supplies a real capture event stream to the renderer.
            visual.picture.click_zoom = style::click_zoom(&scopes);
        }
        clip.visual = Some(visual);
    }
    if clip.take {
        let keys = enabled.keys == Some(true);
        if keys {
            clip.keys = style::keys(&scopes);
        }
        if enabled.camera == Some(true) {
            let style = style::camera(&scopes);
            let (bounds, radius) = layout::camera(size, style, screen, keys);
            clip.camera = Some(Camera {
                bounds,
                radius,
                shadow: style.shadow,
            });
        }
    }
    Ok(clip)
}

fn media(project: &Project, id: MediaId) -> Result<&Media, EvalError> {
    project
        .media
        .iter()
        .find(|media| media.id == id)
        .ok_or(EvalError::Media(id))
}

fn input(
    media: &Media,
    source: Frame,
    step: f64,
    tracks: Vec<(Track, f64)>,
) -> Result<Input, EvalError> {
    if !step.is_finite()
        || step <= 0.0
        || media.probe.frames < 1
        || source < 0
        || source >= media.probe.frames
        || tracks
            .iter()
            .any(|&(_, gain)| !gain.is_finite() || gain < 0.0)
    {
        return Err(EvalError::Range);
    }
    Ok(Input {
        id: media.id,
        source: f64::from(source),
        step,
        frames: f64::from(media.probe.frames),
        tracks,
    })
}

fn visual(
    media: &Media,
    id: TimelineId,
    bounds: Rect,
    framing: Framing,
    window: Option<style::Window>,
) -> Result<Visual, EvalError> {
    let size = [media.probe.width, media.probe.height];
    if size.contains(&0)
        || framing.crop.is_some_and(|r| {
            ![r.x, r.y, r.w, r.h].iter().all(|n| n.is_finite())
                || r.x < 0.0
                || r.y < 0.0
                || r.w <= 0.0
                || r.h <= 0.0
                || r.x + r.w > 1.0 + f64::EPSILON
                || r.y + r.h > 1.0 + f64::EPSILON
        })
    {
        return Err(EvalError::Geometry(media.id));
    }
    let (source, destination) = layout::picture(
        bounds,
        size,
        framing,
        window.map_or(0.0, |style| style.padding),
    );
    if ![source, destination].iter().all(|rect| {
        [rect.x, rect.y, rect.w, rect.h]
            .iter()
            .all(|number| number.is_finite())
            && rect.w > 0.0
            && rect.h > 0.0
    }) {
        return Err(EvalError::Geometry(media.id));
    }
    let mut visual = blank_visual(media.id, id, bounds, window);
    visual.visible = true;
    visual.picture.source = source;
    visual.picture.destination = destination;
    Ok(visual)
}

fn blank_visual(
    media: MediaId,
    item: TimelineId,
    bounds: Rect,
    window: Option<style::Window>,
) -> Visual {
    Visual {
        bounds,
        visible: false,
        picture: Picture {
            item,
            media,
            source_frame: 0.0,
            source: layout::rect(0.0, 0.0, 1.0, 1.0),
            destination: bounds,
            scale: 1.0,
            opacity: 1.0,
            window,
            cursor: None,
            click_zoom: None,
        },
    }
}

fn take_tracks(enabled: &ComponentStates, gains: AudioGains) -> Vec<(Track, f64)> {
    let mut tracks = Vec::new();
    if enabled.mic == Some(true) {
        tracks.push((Track::Mic, gains.mic));
    }
    if enabled.system == Some(true) {
        tracks.push((Track::System, gains.system));
    }
    tracks
}

fn file_tracks(media: &Media) -> Vec<(Track, f64)> {
    media
        .probe
        .audio
        .iter()
        .enumerate()
        .map(|(index, _)| (Track::Imported(index as u32), 1.0))
        .collect()
}

fn overlay_tracks(media: &Media) -> Vec<(Track, f64)> {
    if media.kind != MediaKind::Take {
        return file_tracks(media);
    }
    let booked = |name| {
        media
            .probe
            .audio
            .iter()
            .any(|track| track.name == name)
            .then_some(true)
    };
    take_tracks(
        &ComponentStates {
            mic: booked("mic"),
            system: booked("system"),
            ..ComponentStates::default()
        },
        AudioGains::default(),
    )
}

/// Resolves cue words through the source timeline and applies display edits.
pub fn caption_tokens(
    p: &Project,
    anchor: Anchor,
    ids: &[WordId],
    edits: &[CaptionEdit],
    uppercase: bool,
) -> Result<Vec<CaptionWord>, EvalError> {
    let hash = match anchor {
        Anchor::ClipSource { clip, .. } => p
            .clips
            .find(clip)
            .and_then(|slot| p.clips.data(slot))
            .and_then(|d| d.media)
            .and_then(|id| p.transcripts.iter().find(|t| t.media == id))
            .map(|t| t.hash),
        Anchor::Word {
            transcript_hash, ..
        } => Some(transcript_hash),
        _ => None,
    };
    let mut words = Vec::new();
    for &id in ids {
        let Some(span) = timing::word_span(p, id, hash)? else {
            continue;
        };
        let clip_slot = p
            .clips
            .find(span.clip)
            .ok_or(EvalError::Timing(TimingError::Clip(span.clip)))?;
        let media = p
            .clips
            .data(clip_slot)
            .and_then(|data| data.media)
            .ok_or(EvalError::Timing(TimingError::Slot))?;
        let word = p
            .transcripts
            .iter()
            .find(|transcript| transcript.media == media && transcript.hash == span.hash)
            .and_then(|transcript| transcript.words.iter().find(|word| word.id == id))
            .ok_or(EvalError::Timing(TimingError::Word(id)))?;
        let edit = edits.iter().find(|edit| edit.word == id);
        let text = edit
            .and_then(|edit| edit.text.as_deref())
            .unwrap_or(&word.text);
        let text = if uppercase {
            text.to_uppercase()
        } else {
            text.into()
        };
        words.push(CaptionWord {
            id,
            text,
            emoji: edit.and_then(|edit| edit.emoji.clone()),
            emph: edit.is_some_and(|edit| edit.emph),
            start: span.start,
            end: span.end,
        });
    }
    Ok(words)
}
