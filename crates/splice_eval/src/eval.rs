use super::look::{self, AudioFx, Look, Mix, Num, Rig, Transition, TransitionKind};
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
    end: Frame,
    clips: Vec<PreparedClip>,
    overlays: Vec<PreparedOverlay>,
    titles: Vec<PreparedTitle>,
    captions: Vec<PreparedCaption>,
    motions: Vec<PreparedMotion>,
    sounds: Vec<PreparedSound>,
    shapes: Vec<PreparedShape>,
    /// Timeline spans of speech, sorted and merged, for ducking.
    speech: Vec<(Frame, Frame)>,
    /// LUT paths referenced by `Grade::lut`.
    luts: Vec<String>,
    /// Whether any audible item is soloed (the rest are silent).
    soloed: bool,
    /// Offline audio chains per item.
    fx: Vec<(TimelineId, AudioFx)>,
    /// Items that need the person matte (T18.3).
    cutouts: Vec<(TimelineId, MediaId)>,
}

struct PreparedSound {
    id: ItemId,
    interval: Interval,
    input: Input,
    duck: Option<Duck>,
    mix: Option<Mix>,
}

struct PreparedShape {
    id: ItemId,
    interval: Interval,
    shape: ShapeKind,
    bounds: Rect,
    fill: [u32; 2],
    stroke: u32,
    stroke_width: f64,
    corner: f64,
    rig: Rig,
}

/// An item's timeline interval and linear mapping to its source clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceSpan {
    /// First timeline frame.
    pub start: Frame,
    /// Exclusive timeline end.
    pub end: Frame,
    /// Source frame at the start of the item.
    pub source: f64,
    /// Source frames consumed per timeline frame.
    pub step: f64,
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
    rig: Rig,
    mix: Option<Mix>,
    transition: Option<Transition>,
}

struct Input {
    id: MediaId,
    source: f64,
    step: f64,
    frames: f64,
    tracks: Vec<(Track, f64)>,
    /// Source frames consumed by each timeline frame of a speed ramp (prefix sums).
    map: Option<Vec<f64>>,
}

impl Input {
    fn frame(&self, offset: Frame) -> f64 {
        match &self.map {
            Some(map) if !map.is_empty() => {
                let last = map.len() - 1;
                let index = usize::try_from(offset).unwrap_or(0).min(last);
                let past = f64::from(offset) - index as f64;
                let rate = if last > 0 {
                    map[last] - map[last - 1]
                } else {
                    self.step
                };
                self.source + map[index] + past * rate
            }
            _ => self.source + f64::from(offset) * self.step,
        }
    }

    fn audio(&self, at: f64, item: TimelineId, fade: f64, pan: f64, out: &mut FramePlan) {
        if at >= 0.0 && at < self.frames {
            out.audio
                .extend(self.tracks.iter().map(|&(track, gain)| Audio {
                    item,
                    media: self.id,
                    track,
                    source_frame: at,
                    gain: fade * gain,
                    pan,
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
    fn draw(&self, at: f64, scale: f64, fade: f64, look: Look, out: &mut FramePlan) {
        if let Some(window) = self.picture.window
            && window.background != Background::None
        {
            out.layers.push(Layer::Background {
                bounds: self.bounds,
                fill: window.background,
                opacity: fade * look.opacity,
            });
        }
        if self.visible {
            out.layers.push(Layer::Picture(Picture {
                source_frame: at,
                scale,
                opacity: fade,
                look,
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
    rig: Rig,
    mix: Option<Mix>,
}

struct PreparedTitle {
    id: ItemId,
    interval: Interval,
    text: String,
    rig: Rig,
}

struct PreparedMotion {
    interval: Interval,
    from: f64,
    to: f64,
    focus: Option<[f64; 2]>,
    ease: bool,
}

struct PreparedCaption {
    id: ItemId,
    interval: Interval,
    style: CaptionStyle,
    words: Vec<CaptionWord>,
    rig: Rig,
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

    fn add_breakpoints(self, cuts: &mut Vec<Frame>) {
        cuts.extend([self.start, self.end]);
        let start = i64::from(self.start);
        let end = i64::from(self.end);
        let fade_in = i64::from(self.fade_in.max(0));
        let fade_out = i64::from(self.fade_out.max(0));
        let mut pivot = |at: i64| {
            cuts.extend(
                [at, at + 1]
                    .into_iter()
                    .filter(|&at| at > start && at < end)
                    .map(|at| at as Frame),
            );
        };
        if fade_in > 0 {
            pivot(start + fade_in - 1);
        }
        if fade_out > 0 {
            pivot(end - fade_out);
        }
        if fade_in > 0 && fade_out > 0 && fade_in + fade_out > end - start + 1 {
            pivot((fade_in * end + fade_out * (start - 1)) / (fade_in + fade_out));
        }
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
            end: 0,
            clips: Vec::with_capacity(project.clips.order().len()),
            overlays: Vec::new(),
            titles: Vec::new(),
            captions: Vec::new(),
            motions: Vec::new(),
            sounds: Vec::new(),
            shapes: Vec::new(),
            speech: Vec::new(),
            luts: Vec::new(),
            soloed: false,
            fx: Vec::new(),
            cutouts: Vec::new(),
        };
        let mut end = 0;
        for &slot in project.clips.order() {
            let clip = prepare_clip(project, slot, size, &mut result)?;
            if clip.interval.start < end {
                return Err(EvalError::Range);
            }
            end = clip.interval.end;
            result.clips.push(clip);
        }
        result.end = end;
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
        result.soloed = result
            .clips
            .iter()
            .filter_map(|c| c.mix.as_ref())
            .any(|m| m.solo)
            || result
                .overlays
                .iter()
                .filter_map(|o| o.mix.as_ref())
                .any(|m| m.solo)
            || result
                .sounds
                .iter()
                .filter_map(|s| s.mix.as_ref())
                .any(|m| m.solo);
        if result.sounds.iter().any(|sound| sound.duck.is_some()) {
            result.speech = speech(project, &result.clips)?;
        }
        Ok(result)
    }

    /// Returns the ordered boundaries where the visible stack or its linear ramps can change.
    pub fn breakpoints(&self) -> Vec<Frame> {
        let mut cuts = vec![0, self.end];
        for clip in &self.clips {
            clip.interval.add_breakpoints(&mut cuts);
        }
        for overlay in &self.overlays {
            overlay.interval.add_breakpoints(&mut cuts);
            let input = &overlay.input;
            let audio_end = f64::from(overlay.interval.start)
                + ((input.frames - input.source) / input.step).ceil();
            if audio_end < f64::from(overlay.interval.end) {
                cuts.push(audio_end as Frame);
            }
        }
        for title in &self.titles {
            title.interval.add_breakpoints(&mut cuts);
        }
        for caption in &self.captions {
            caption.interval.add_breakpoints(&mut cuts);
            cuts.extend(
                caption
                    .words
                    .iter()
                    .map(|word| word.start)
                    .filter(|&at| caption.interval.contains(at)),
            );
        }
        for sound in &self.sounds {
            sound.interval.add_breakpoints(&mut cuts);
            if let Some(duck) = sound.duck {
                let inside = |at: &Frame| sound.interval.contains(*at);
                for &(start, end) in &self.speech {
                    cuts.extend(
                        [start - duck.attack, start, end, end + duck.release]
                            .into_iter()
                            .filter(inside),
                    );
                }
            }
        }
        for clip in &self.clips {
            clip.rig.breakpoints(clip.interval.start, &mut cuts);
            if let Some(mix) = &clip.mix {
                mix.breakpoints(clip.interval.start, &mut cuts);
            }
            if let Some(transition) = clip.transition {
                let start = clip.interval.start - transition.length / 2;
                cuts.extend([
                    start,
                    start + transition.length / 2,
                    start + transition.length,
                ]);
            }
            if clip.input.as_ref().is_some_and(|input| input.map.is_some()) {
                // Speed ramps lower as piecewise-constant rates.
                cuts.extend((clip.interval.start..clip.interval.end).step_by(2));
            }
        }
        for overlay in &self.overlays {
            overlay.rig.breakpoints(overlay.interval.start, &mut cuts);
            if let Some(mix) = &overlay.mix {
                mix.breakpoints(overlay.interval.start, &mut cuts);
            }
        }
        for sound in &self.sounds {
            if let Some(mix) = &sound.mix {
                mix.breakpoints(sound.interval.start, &mut cuts);
            }
        }
        for title in &self.titles {
            title.rig.breakpoints(title.interval.start, &mut cuts);
        }
        for shape in &self.shapes {
            shape.interval.add_breakpoints(&mut cuts);
            shape.rig.breakpoints(shape.interval.start, &mut cuts);
        }
        for caption in &self.captions {
            caption.rig.breakpoints(caption.interval.start, &mut cuts);
        }
        for motion in &self.motions {
            motion.interval.add_breakpoints(&mut cuts);
            if motion.ease {
                // Renderers ramp linearly between cuts: sample the eased curve every 4 frames.
                cuts.extend((motion.interval.start..motion.interval.end).step_by(4));
            }
        }
        cuts.retain(|&at| at >= 0 && at <= self.end);
        cuts.sort_unstable();
        cuts.dedup();
        cuts
    }

    /// A prepared LUT path.
    pub fn lut(&self, index: u16) -> Option<&str> {
        self.luts.get(usize::from(index)).map(String::as_str)
    }

    /// The offline audio chain of a clip or item, when it has one.
    pub fn audio_fx(&self, item: TimelineId) -> Option<AudioFx> {
        self.fx
            .iter()
            .find_map(|&(id, fx)| (id == item).then_some(fx))
    }

    /// Items with a person cutout and their media (the matte comes from the analysis store).
    pub fn cutouts(&self) -> &[(TimelineId, MediaId)] {
        &self.cutouts
    }

    /// The transition into a clip, if it has one.
    pub fn transition(&self, clip: ClipId) -> Option<Transition> {
        self.clips
            .iter()
            .find(|prepared| prepared.id == clip)
            .and_then(|prepared| prepared.transition)
    }

    /// The speed-ramp source offset table of a clip (source frames consumed per timeline frame).
    pub fn speed_map(&self, clip: ClipId) -> Option<&[f64]> {
        self.clips
            .iter()
            .find(|prepared| prepared.id == clip)
            .and_then(|prepared| prepared.input.as_ref()?.map.as_deref())
    }

    /// Returns the prepared source clock for a clip or a media overlay.
    pub fn source_span(&self, item: TimelineId) -> Option<SourceSpan> {
        let (interval, input) = self.source(item)?;
        Some(SourceSpan {
            start: interval.start,
            end: interval.end,
            source: input.source,
            step: input.step,
        })
    }

    /// Returns the track gain before applying the item's fade.
    pub fn audio_gain(&self, item: TimelineId, track: Track) -> Option<f64> {
        self.source(item)?
            .1
            .tracks
            .iter()
            .find_map(|&(id, gain)| (id == track).then_some(gain))
    }

    fn source(&self, item: TimelineId) -> Option<(Interval, &Input)> {
        let (interval, input) = match item {
            TimelineId::Clip(id) => {
                let clip = self.clips.iter().find(|clip| clip.id == id)?;
                (clip.interval, clip.input.as_ref()?)
            }
            TimelineId::Item(id) => {
                if let Some(sound) = self.sounds.iter().find(|sound| sound.id == id) {
                    (sound.interval, &sound.input)
                } else {
                    let overlay = self
                        .overlays
                        .iter()
                        .find(|overlay| overlay.visual.picture.item == item)?;
                    (overlay.interval, &overlay.input)
                }
            }
            TimelineId::Marker(_) => return None,
        };
        Some((interval, input))
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
            .chain(self.sounds.iter().map(|sound| sound.input.tracks.len()))
            .sum::<usize>();
        FramePlan {
            layers: Vec::with_capacity(
                12 + self.overlays.len() * 2
                    + self.titles.len()
                    + self.captions.len()
                    + self.shapes.len(),
            ),
            audio: Vec::with_capacity(2 * clip_tracks + overlay_tracks),
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
        if let Some((from, to, progress, transition)) = self.transition_at(at) {
            let (mark, audio_mark) = (out.layers.len(), out.audio.len());
            self.draw_clip(&self.clips[from], at, out);
            let (mid, audio_mid) = (out.layers.len(), out.audio.len());
            self.draw_clip(&self.clips[to], at, out);
            self.blend(
                transition,
                progress,
                [mark, mid],
                [audio_mark, audio_mid],
                out,
            );
        } else if let Some(clip) = clip {
            self.draw_clip(clip, at, out);
        }
        self.draw_overlays(at, clip.is_some_and(|clip| clip.take), false, out);
        for sound in self
            .sounds
            .iter()
            .filter(|sound| sound.interval.contains(at))
        {
            let local = f64::from(at - sound.interval.start);
            let source = sound.input.frame(at - sound.interval.start);
            let (gain, pan) = self.mix(sound.mix.as_ref(), local);
            let gain = sound.interval.alpha(at) * self.duck_gain(sound.duck, at) * gain;
            sound
                .input
                .audio(source, TimelineId::Item(sound.id), gain, pan, out);
        }
        let canvas = self.size.map(f64::from);
        for shape in self
            .shapes
            .iter()
            .filter(|shape| shape.interval.contains(at))
        {
            out.layers.push(Layer::Shape {
                item: shape.id,
                shape: shape.shape,
                bounds: shape.bounds,
                fill: shape.fill[0],
                fill2: shape.fill[1],
                stroke: shape.stroke,
                stroke_width: shape.stroke_width,
                corner: shape.corner,
                opacity: shape.interval.alpha(at),
                look: shape.rig.look(f64::from(at - shape.interval.start), canvas),
            });
        }
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
                look: title.rig.look(f64::from(at - title.interval.start), canvas),
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
                look: caption
                    .rig
                    .look(f64::from(at - caption.interval.start), canvas),
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
        let local = f64::from(frame - clip.interval.start);
        let source = input.frame(frame - clip.interval.start);
        let opacity = clip.interval.alpha(frame);
        let look = clip.rig.look(local, self.size.map(f64::from));
        let picture_source = source.clamp(0.0, (input.frames - 1.0).max(0.0));
        if let Some(visual) = &clip.visual {
            let motion = self
                .motions
                .iter()
                .find(|motion| motion.interval.contains(frame));
            let zoom = motion.map_or(1.0, |motion| {
                let mut progress = f64::from(frame - motion.interval.start)
                    / f64::from((motion.interval.end - motion.interval.start - 1).max(1));
                if motion.ease {
                    progress = progress * progress * (3.0 - 2.0 * progress);
                }
                motion.from + (motion.to - motion.from) * progress
            });
            visual.draw(picture_source, zoom, 1.0, look, out);
            if let (Some(motion), Some(Layer::Picture(picture))) = (motion, out.layers.last_mut()) {
                picture.focus = motion.focus;
            }
        }
        let (gain, pan) = self.mix(clip.mix.as_ref(), local);
        input.audio(source, TimelineId::Clip(clip.id), opacity * gain, pan, out);
        if clip.take {
            self.draw_overlays(frame, true, true, out);
        }
        if let Some(camera) = &clip.camera {
            out.layers.push(Layer::Camera {
                clip: clip.id,
                media: input.id,
                source_frame: picture_source,
                bounds: camera.bounds,
                radius: camera.radius,
                shadow: camera.shadow,
            });
        }
        if let Some(style) = clip.keys {
            out.layers.push(Layer::Keystrokes {
                clip: clip.id,
                media: input.id,
                source_frame: picture_source,
                style,
            });
        }
        if opacity < 1.0 {
            out.layers.push(Layer::Fade {
                opacity: 1.0 - opacity,
                color: 0,
            });
        }
    }

    fn draw_overlays(&self, frame: Frame, on_take: bool, behind: bool, out: &mut FramePlan) {
        for overlay in self.overlays.iter().filter(|overlay| {
            (overlay.behind && on_take) == behind && overlay.interval.contains(frame)
        }) {
            let opacity = overlay.interval.alpha(frame);
            let local = f64::from(frame - overlay.interval.start);
            let source = overlay.input.frame(frame - overlay.interval.start);
            overlay.visual.draw(
                source.min((overlay.input.frames - 1.0).max(0.0)),
                1.0,
                opacity,
                overlay.rig.look(local, self.size.map(f64::from)),
                out,
            );
            let (gain, pan) = self.mix(overlay.mix.as_ref(), local);
            overlay.input.audio(
                source,
                overlay.visual.picture.item,
                opacity * gain,
                pan,
                out,
            );
        }
    }

    /// Keyed gain and pan of an audible item, silenced when another item is soloed.
    fn mix(&self, mix: Option<&Mix>, local: f64) -> (f64, f64) {
        match mix {
            Some(mix) if !(self.soloed && !mix.solo) => (mix.gain(local), mix.pan(local)),
            Some(_) => (0.0, 0.0),
            None if self.soloed => (0.0, 0.0),
            None => (1.0, 0.0),
        }
    }

    /// The clips a transition blends at `at`: outgoing index, incoming index, progress 0…1.
    fn transition_at(&self, at: Frame) -> Option<(usize, usize, f64, Transition)> {
        let index = self.clips.partition_point(|clip| clip.interval.start <= at);
        for to in [index, index.saturating_sub(1)] {
            let Some(clip) = self.clips.get(to).filter(|_| to > 0) else {
                continue;
            };
            let Some(transition) = clip.transition else {
                continue;
            };
            if self.clips[to - 1].interval.end != clip.interval.start {
                continue;
            }
            let start = clip.interval.start - transition.length / 2;
            if at >= start && at < start + transition.length {
                let progress = (f64::from(at - start) + 0.5) / f64::from(transition.length);
                return Some((to - 1, to, progress, transition));
            }
        }
        None
    }

    /// Applies a transition to the outgoing layers `[a, b)` and incoming layers `[b, ..)`.
    fn blend(
        &self,
        transition: Transition,
        t: f64,
        [a, b]: [usize; 2],
        [audio_a, audio_b]: [usize; 2],
        out: &mut FramePlan,
    ) {
        let [w, h] = self.size.map(f64::from);
        let [dx, dy] = transition.direction;
        let quarter = std::f64::consts::FRAC_PI_2;
        for (index, audio) in out.audio.iter_mut().enumerate().skip(audio_a) {
            audio.gain *= if index < audio_b {
                (t * quarter).cos()
            } else {
                (t * quarter).sin()
            };
        }
        let edit = |layer: &mut Layer, f: &dyn Fn(&mut Look, &mut f64)| match layer {
            Layer::Picture(picture) => f(&mut picture.look, &mut picture.opacity),
            Layer::Background { opacity, .. } => {
                let mut look = Look::default();
                f(&mut look, opacity);
                *opacity *= look.opacity;
            }
            _ => {}
        };
        match transition.kind {
            TransitionKind::Dissolve | TransitionKind::Zoom => {
                let zoom = transition.kind == TransitionKind::Zoom;
                for (index, layer) in out.layers.iter_mut().enumerate().skip(a) {
                    let incoming = index >= b;
                    edit(layer, &|look, _| {
                        if incoming {
                            look.opacity *= t;
                            if zoom {
                                look.scale *= 1.15 - 0.15 * t;
                            }
                        } else if zoom {
                            look.scale *= 1.0 + 0.15 * t;
                        }
                    });
                }
            }
            TransitionKind::Dip => {
                let color = transition.color;
                if t < 0.5 {
                    out.layers.truncate(b);
                    out.layers.push(Layer::Fade {
                        opacity: (2.0 * t).min(1.0),
                        color,
                    });
                } else {
                    out.layers.drain(a..b);
                    out.layers.push(Layer::Fade {
                        opacity: (2.0 * (1.0 - t)).min(1.0),
                        color,
                    });
                }
            }
            TransitionKind::Wipe => {
                let clip = [
                    if dx < 0.0 { w * (1.0 - t) } else { 0.0 },
                    if dy < 0.0 { h * (1.0 - t) } else { 0.0 },
                    if dx != 0.0 { w * t } else { w },
                    if dy != 0.0 { h * t } else { h },
                ];
                for layer in out.layers.iter_mut().skip(b) {
                    edit(layer, &|look, _| look.clip = Some(clip));
                }
            }
            TransitionKind::Push | TransitionKind::Slide => {
                let push = transition.kind == TransitionKind::Push;
                for (index, layer) in out.layers.iter_mut().enumerate().skip(a) {
                    let incoming = index >= b;
                    let shift = if incoming {
                        [-dx * (1.0 - t) * w, -dy * (1.0 - t) * h]
                    } else if push {
                        [dx * t * w, dy * t * h]
                    } else {
                        [0.0, 0.0]
                    };
                    match layer {
                        Layer::Background { bounds, .. } | Layer::Camera { bounds, .. } => {
                            bounds.x += shift[0];
                            bounds.y += shift[1];
                        }
                        _ => edit(layer, &|look, _| {
                            look.offset[0] += shift[0];
                            look.offset[1] += shift[1];
                        }),
                    }
                }
            }
        }
    }

    /// The sound's gain under its ducking at `at`: ramps down over `attack` before speech and
    /// back up over `release` after it.
    fn duck_gain(&self, duck: Option<Duck>, at: Frame) -> f64 {
        let Some(duck) = duck else { return 1.0 };
        let first = self
            .speech
            .partition_point(|&(_, end)| end + duck.release <= at);
        let mut depth: f64 = 0.0;
        for &(start, end) in self.speech[first..].iter().take(2) {
            let ramp = if at >= start && at < end {
                1.0
            } else if at < start && at >= start - duck.attack {
                f64::from(at - (start - duck.attack)) / f64::from(duck.attack.max(1))
            } else if at >= end && at < end + duck.release {
                1.0 - f64::from(at - end) / f64::from(duck.release.max(1))
            } else {
                0.0
            };
            depth = depth.max(ramp);
        }
        1.0 - (1.0 - duck.amount) * depth
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
        self.end = self.end.max(interval.end);
        let lane = &p.lanes[rank].effects;
        let local = &data.appearance.effects;
        let inherited: [&[Effect]; 2] = [&p.effects, lane];
        let audio_scopes: [&[Effect]; 3] = [&p.effects, lane, local];
        let shifts = [-start, -start, 0];
        let rig = Rig::new(&inherited, &shifts, local, &mut self.luts);
        let fx = AudioFx::new(&audio_scopes);
        if fx != AudioFx::default() {
            self.fx.push((TimelineId::Item(id), fx));
        }
        match &data.kind {
            ItemKind::Title { text } => self.titles.push(PreparedTitle {
                id,
                interval,
                text: text.clone(),
                rig,
            }),
            ItemKind::Shape { shape, rect } => {
                let style = look::shape_style(local);
                let [w, h] = size;
                self.shapes.push(PreparedShape {
                    id,
                    interval,
                    shape: *shape,
                    bounds: layout::rect(rect.x * w, rect.y * h, rect.w * w, rect.h * h),
                    fill: [style.0, style.1],
                    stroke: style.2,
                    stroke_width: style.3 * w.min(h),
                    corner: style.4 * (rect.w * w).min(rect.h * h).abs(),
                    rig,
                });
            }
            ItemKind::Motion {
                from,
                to,
                focus,
                ease,
            } => {
                if !from.is_finite() || !to.is_finite() || *from <= 0.0 || *to <= 0.0 {
                    return Err(EvalError::Range);
                }
                self.motions.push(PreparedMotion {
                    interval,
                    from: *from,
                    to: *to,
                    focus: *focus,
                    ease: *ease,
                });
            }
            ItemKind::Sound(sound) => {
                let media = media(p, sound.media)?;
                if !sound.volume.is_finite() || sound.volume < 0.0 {
                    return Err(EvalError::Range);
                }
                let tracks = file_tracks(media)
                    .into_iter()
                    .map(|(track, gain)| (track, gain * sound.volume))
                    .collect();
                self.sounds.push(PreparedSound {
                    id,
                    interval,
                    input: input(
                        media,
                        sound.source,
                        media.probe.fps / f64::from(p.fps),
                        tracks,
                    )?,
                    duck: sound.duck,
                    mix: Mix::new(&audio_scopes, &shifts),
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
                if rig.cutout() {
                    self.cutouts.push((TimelineId::Item(id), media.id));
                }
                self.overlays.push(PreparedOverlay {
                    interval,
                    input,
                    visual,
                    behind: overlay.behind_presenter,
                    rig,
                    mix: Mix::new(&audio_scopes, &shifts),
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
                            rig,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

fn prepare_clip(
    p: &Project,
    slot: Slot,
    size: [f64; 2],
    evaluator: &mut Evaluator,
) -> Result<PreparedClip, EvalError> {
    let id = p.clips.id(slot).ok_or(EvalError::Range)?;
    let data = p.clips.data(slot).ok_or(EvalError::Range)?;
    let timing = p.clips.timing(slot).ok_or(EvalError::Range)?;
    let interval = Interval::new(timing.start, timing.length, &data.appearance)?;
    let local = &data.appearance.effects;
    let inherited: [&[Effect]; 2] = [&p.effects, &p.spine.effects];
    let audio_scopes: [&[Effect]; 3] = [&p.effects, &p.spine.effects, local];
    let shifts = [-timing.start, -timing.start, 0];
    let mut clip = PreparedClip {
        id,
        interval,
        input: None,
        take: false,
        visual: None,
        camera: None,
        keys: None,
        hidden_keys: data.hidden_keys.clone(),
        rig: Rig::new(&inherited, &shifts, local, &mut evaluator.luts),
        mix: Mix::new(&audio_scopes, &shifts),
        transition: look::transition(local),
    };
    let fx = AudioFx::new(&audio_scopes);
    if fx != AudioFx::default() {
        evaluator.fx.push((TimelineId::Clip(id), fx));
    }
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
    let mut input = input(media, timing.source, timing::source_rate(p, slot)?, tracks)?;
    if let Some((rate, _)) = look::speed(local) {
        input.map = Some(speed_map(&rate, input.step, timing.length));
    }
    if input.frame(timing.length) > input.frames + 0.000_001 {
        return Err(EvalError::Range);
    }
    if clip.rig.cutout() && media.kind != MediaKind::Audio {
        evaluator.cutouts.push((TimelineId::Clip(id), media.id));
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

/// Source frames consumed at each timeline frame 0..=length of a speed-ramped clip.
fn speed_map(rate: &Num, step: f64, length: Frame) -> Vec<f64> {
    let mut map = Vec::with_capacity(usize::try_from(length).unwrap_or(0) + 1);
    let mut sum = 0.0;
    map.push(0.0);
    // Trapezoids at quarter frames, carried forward: O(length).
    let mut previous = rate.at(0.0).max(0.0);
    for frame in 0..length.max(0) {
        for quarter in 1..=4 {
            let next = rate
                .at(f64::from(frame) + f64::from(quarter) * 0.25)
                .max(0.0);
            sum += (previous + next) * 0.125 * step;
            previous = next;
        }
        map.push(sum);
    }
    map
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
        map: None,
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
            focus: None,
            opacity: 1.0,
            window,
            cursor: None,
            click_zoom: None,
            look: Look::default(),
        },
    }
}

/// Speech on the timeline, for ducking: transcript words, or else clips with a microphone.
/// Spans closer than 12 frames merge.
fn speech(p: &Project, clips: &[PreparedClip]) -> Result<Vec<(Frame, Frame)>, EvalError> {
    let mut spans = Vec::new();
    for transcript in &p.transcripts {
        for word in &transcript.words {
            if let Some(span) = timing::word_span(p, word.id, Some(transcript.hash))? {
                spans.push((span.start, span.end));
            }
        }
    }
    if spans.is_empty() {
        spans.extend(
            clips
                .iter()
                .filter(|clip| {
                    clip.input
                        .as_ref()
                        .is_some_and(|input| input.tracks.iter().any(|&(t, _)| t == Track::Mic))
                })
                .map(|clip| (clip.interval.start, clip.interval.end)),
        );
    }
    spans.sort_unstable();
    let mut merged: Vec<(Frame, Frame)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start - last.1 <= 12 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    Ok(merged)
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
