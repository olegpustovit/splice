use super::look::{self, AudioFx, Look, Mix, Num, Rig, Transition, TransitionKind};
use super::{
    Audio, Background, CaptionStyle, CaptionWord, FREE_TEXT, FramePlan, Group, Keys, Layer,
    Picture, Shift, TextRun, Track, layout, particles, style,
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
    /// Zoom segments while preparing; solved into `camera` at the end.
    zooms: Vec<crate::camera::Segment>,
    /// The timeline camera, one view per frame (D12).
    camera: Vec<crate::View>,
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
    /// One paint order across kinds (T42.4): (z, kind, index) with kind 0 overlays, 1 shapes,
    /// 2 titles, 3 captions, sorted by z then kind (stable, so lanes keep their order).
    paint: Vec<(i32, u8, usize)>,
    /// Lane transforms by lane index, applied as parents of the lane's items (T42.4).
    lane_parents: Vec<Option<Rig>>,
    /// The spine's transform, the parent of spine pictures.
    spine_parent: Option<Rig>,
    /// The project's transform, the parent of everything drawn but the backdrop.
    project_parent: Option<Rig>,
}

/// Where an item paints: its z and lane index (the parent transform).
#[derive(Clone, Copy)]
struct Place {
    z: i32,
    lane: usize,
}

struct PreparedSound {
    id: ItemId,
    interval: Interval,
    input: Input,
    duck: Option<Duck>,
    mix: Option<Mix>,
}

struct PreparedShape {
    place: Place,
    id: ItemId,
    interval: Interval,
    shape: ShapeKind,
    bounds: Rect,
    fill: [u32; 2],
    stroke: u32,
    stroke_width: f64,
    corner: f64,
    rig: Rig,
    /// A progress bar or ring (T45.4): value, track color and opacity.
    progress: Option<(Num, u32, f64)>,
    /// A particle emitter (T45.4): it draws its particles instead of the shape.
    particles: Option<particles::Emitter>,
}

impl PreparedShape {
    /// Layers it adds to every frame it is on.
    fn layers(&self) -> usize {
        match (&self.particles, &self.progress) {
            (Some(emitter), _) => emitter.len(),
            (None, Some(_)) => 2,
            (None, None) => 1,
        }
    }
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
    /// The cursor effect's `clickSound` (T46.2).
    click_sound: bool,
    visual: Option<Visual>,
    camera: Option<Camera>,
    keys: Option<Keys>,
    hidden_keys: Vec<u32>,
    /// The static crop (the whole source when uncropped): the space of zoom focus points.
    crop: Rect,
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

/// A take's camera bubble, laid out per frame (keys, layouts, shrink when zoomed).
struct Camera {
    style: style::CameraStyle,
    screen: bool,
    keys: bool,
    /// A layout switch's ease length in frames (0.3 s).
    ease: f64,
}

struct Visual {
    bounds: Rect,
    visible: bool,
    picture: Picture,
}

impl Visual {
    /// `card`: an overlay's backing frames its card and moves, scales and fades with it; a
    /// spine clip's backing is the canvas backdrop and never borrows the clip's look (H7, D14).
    fn draw(&self, at: f64, fade: f64, look: Look, card: bool, out: &mut FramePlan) {
        if let Some(window) = self.picture.window
            && window.background != Background::None
        {
            let (bounds, opacity) = if card {
                // Scaled about the picture's own pivot, so card and backing stay aligned.
                let d = self.picture.destination;
                let pivot = [d.x + look.anchor[0] * d.w, d.y + look.anchor[1] * d.h];
                let b = self.bounds;
                let bounds = Rect {
                    x: pivot[0] + (b.x - pivot[0]) * look.scale + look.offset[0],
                    y: pivot[1] + (b.y - pivot[1]) * look.scale + look.offset[1],
                    w: b.w * look.scale,
                    h: b.h * look.scale,
                };
                (bounds, fade * look.opacity)
            } else {
                (self.bounds, fade)
            };
            out.layers.push(Layer::Background {
                bounds,
                fill: window.background,
                opacity,
            });
        }
        if self.visible {
            out.layers.push(Layer::Picture(Picture {
                source_frame: at,
                opacity: fade,
                look,
                ..self.picture
            }));
        }
    }
}

struct PreparedOverlay {
    place: Place,
    interval: Interval,
    input: Input,
    visual: Visual,
    behind: bool,
    rig: Rig,
    mix: Option<Mix>,
}

struct PreparedTitle {
    place: Place,
    id: ItemId,
    interval: Interval,
    text: String,
    rig: Rig,
    style: Option<style::TextStyle>,
    text_box: Option<style::TextBox>,
    /// A counter's text per run of frames (T45.4); empty for plain titles.
    runs: Vec<TextRun>,
}

struct PreparedCaption {
    place: Place,
    id: ItemId,
    interval: Interval,
    style: CaptionStyle,
    words: Vec<CaptionWord>,
    rig: Rig,
    text: Option<style::TextStyle>,
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
    /// Zooms that follow the cursor hold their framing: pass the takes' pointers to
    /// `prepare_with` wherever they are readable.
    pub fn prepare(project: &Project) -> Result<Self, EvalError> {
        Self::prepare_with(project, &[])
    }

    /// `prepare` with the recorded pointers of takes (T46.2): zooms with `focus:"cursor"` follow
    /// the cursor exactly as the renderer draws it from the same samples.
    pub fn prepare_with(
        project: &Project,
        pointers: &[crate::cursor::Recorded],
    ) -> Result<Self, EvalError> {
        match super::compound::flatten(project)? {
            Some(flat) => Self::prepare_flat(&flat, pointers),
            None => Self::prepare_flat(project, pointers),
        }
    }

    fn prepare_flat(
        project: &Project,
        pointers: &[crate::cursor::Recorded],
    ) -> Result<Self, EvalError> {
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
            zooms: Vec::new(),
            camera: Vec::new(),
            sounds: Vec::new(),
            shapes: Vec::new(),
            speech: Vec::new(),
            luts: Vec::new(),
            soloed: false,
            fx: Vec::new(),
            cutouts: Vec::new(),
            paint: Vec::new(),
            lane_parents: project
                .lanes
                .iter()
                .map(|l| Rig::parent(&l.effects))
                .collect(),
            spine_parent: Rig::parent(&project.spine.effects),
            project_parent: Rig::parent(&project.effects),
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
        let places = [
            result.overlays.iter().map(|o| o.place).collect::<Vec<_>>(),
            result.shapes.iter().map(|o| o.place).collect(),
            result.titles.iter().map(|o| o.place).collect(),
            result.captions.iter().map(|o| o.place).collect(),
        ];
        for (kind, list) in places.iter().enumerate() {
            result
                .paint
                .extend(list.iter().enumerate().map(|(i, p)| (p.z, kind as u8, i)));
        }
        result.paint.sort_by_key(|&(z, kind, _)| (z, kind));
        // Where segments overlap, the later start wins (sort is stable for equal starts).
        // No zoom segments: no table (pictures keep View::IDENTITY), nothing solved.
        let mut zooms = std::mem::take(&mut result.zooms);
        if !zooms.is_empty() {
            zooms.sort_by_key(|segment| segment.start);
            let cursor = result.follow_points(project, &zooms, pointers);
            result.camera = crate::camera::solve(&zooms, result.end, project.fps, &cursor);
        }
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
        // Keyed looks are sampled per frame by the renderer; only audio ramps and stack
        // changes need cuts.
        for clip in &self.clips {
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
        }
        for overlay in &self.overlays {
            if let Some(mix) = &overlay.mix {
                mix.breakpoints(overlay.interval.start, &mut cuts);
            }
        }
        for sound in &self.sounds {
            if let Some(mix) = &sound.mix {
                mix.breakpoints(sound.interval.start, &mut cuts);
            }
        }
        for shape in &self.shapes {
            shape.interval.add_breakpoints(&mut cuts);
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

    /// Timeline frames an item is drawn in, including transition overlap past a clip's edges.
    pub fn extent(&self, item: TimelineId) -> Option<[Frame; 2]> {
        let TimelineId::Clip(id) = item else {
            let TimelineId::Item(id) = item else {
                return None;
            };
            let interval = self
                .titles
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.interval)
                .or_else(|| {
                    self.captions
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.interval)
                })
                .or_else(|| self.source(item).map(|(interval, _)| interval))?;
            return Some([interval.start, interval.end]);
        };
        let index = self.clips.iter().position(|clip| clip.id == id)?;
        let clip = &self.clips[index];
        let mut extent = [clip.interval.start, clip.interval.end];
        if let Some(t) = clip.transition
            && index > 0
            && self.clips[index - 1].interval.end == clip.interval.start
        {
            extent[0] -= t.length / 2;
        }
        if let Some(next) = self.clips.get(index + 1)
            && let Some(t) = next.transition
            && next.interval.start == clip.interval.end
        {
            extent[1] += t.length - t.length / 2;
        }
        Some(extent)
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

    /// The drawn cursor as a content point at each frame a following zoom covers (`None`
    /// elsewhere, or where no take cursor shows): the table the renderer draws from, its loop,
    /// mapped through the clip's keyed crop into the space `zoom.focus` uses.
    fn follow_points(
        &self,
        p: &Project,
        zooms: &[crate::camera::Segment],
        pointers: &[crate::cursor::Recorded],
    ) -> Vec<Option<[f64; 2]>> {
        if pointers.is_empty() || !zooms.iter().any(|z| z.follow) {
            return Vec::new();
        }
        let mut points = vec![None; usize::try_from(self.end).unwrap_or(0)];
        // One table per take and treatment, as the renderer keeps them.
        let mut tables: Vec<(MediaId, crate::cursor::Motion, crate::cursor::Track)> = Vec::new();
        let size = self.size.map(f64::from);
        for zoom in zooms.iter().filter(|z| z.follow) {
            for frame in zoom.start.max(0)..zoom.end.min(self.end) {
                let index = self.clips.partition_point(|c| c.interval.start <= frame);
                let Some(clip) = index
                    .checked_sub(1)
                    .and_then(|i| self.clips.get(i))
                    .filter(|c| c.take && c.interval.contains(frame))
                else {
                    continue;
                };
                let (Some(input), Some(visual)) = (&clip.input, &clip.visual) else {
                    continue;
                };
                let Some(recorded) = pointers.iter().find(|r| r.media == input.id) else {
                    continue;
                };
                let Ok(media) = media(p, input.id) else {
                    continue;
                };
                let style = visual.picture.cursor;
                let motion = style.map_or_else(crate::cursor::Motion::default, |c| c.motion);
                let table = match tables
                    .iter()
                    .position(|(m, known, _)| *m == input.id && *known == motion)
                {
                    Some(i) => i,
                    None => {
                        let marks = media.take.as_ref().map_or(&[][..], |t| &t.pointer);
                        let samples = crate::cursor::with_marks(&recorded.samples, marks);
                        let track = crate::cursor::Track::build(&samples, recorded.size, motion);
                        tables.push((input.id, motion, track));
                        tables.len() - 1
                    }
                };
                let track = &tables[table].2;
                let fps = media.probe.fps.max(1.0);
                let at = |frame: Frame| track.at(input.frame(frame - clip.interval.start) / fps);
                let Some(mut point) = at(frame) else {
                    continue;
                };
                if let (Some(style), Some(first)) = (style, at(clip.interval.start)) {
                    let left = f64::from(clip.interval.end - 1 - frame);
                    point = crate::cursor::looped(point, first, left, style.loop_frames);
                }
                let local = f64::from(frame - clip.interval.start);
                let picture = Picture {
                    look: clip.rig.look(local, size),
                    ..visual.picture
                };
                let dims = [f64::from(media.probe.width), f64::from(media.probe.height)];
                let [x, y, w, h] = picture.content(dims);
                if w > 0.0 && h > 0.0 {
                    let content = [
                        ((point.x - x) / w).clamp(0.0, 1.0),
                        ((point.y - y) / h).clamp(0.0, 1.0),
                    ];
                    // In the space the segment's camera frames (see `canvas_point`).
                    points[frame as usize] = Some(if zoom.mode.zooms_content() {
                        content
                    } else {
                        let d = picture.destination;
                        [
                            (d.x + content[0] * d.w) / size[0],
                            (d.y + content[1] * d.h) / size[1],
                        ]
                        .map(|v| v.clamp(0.0, 1.0))
                    });
                }
            }
        }
        points
    }

    /// The timeline camera at `at` (identity outside the timeline or without zooms). The
    /// renderer reads the previous frame's view for motion blur.
    pub fn view(&self, at: Frame) -> crate::View {
        usize::try_from(at)
            .ok()
            .and_then(|i| self.camera.get(i))
            .copied()
            .unwrap_or(crate::View::IDENTITY)
    }

    /// Whether a take clip mixes a click into the audio at each press (`clickSound`).
    pub fn click_sound(&self, clip: ClipId) -> bool {
        self.clips.iter().any(|c| c.id == clip && c.click_sound)
    }

    /// The camera mode in effect at `at` (the zoom segment's, or `Auto` outside segments).
    pub fn camera_mode(&self, at: Frame) -> CameraMode {
        usize::try_from(at)
            .ok()
            .and_then(|i| self.camera.get(i))
            .map_or(CameraMode::Auto, |view| view.mode)
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
                    + self.shapes.iter().map(PreparedShape::layers).sum::<usize>(),
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
        out.group = None;
        out.shift = None;
        let index = self.clips.partition_point(|clip| clip.interval.start <= at);
        let clip = index
            .checked_sub(1)
            .and_then(|i| self.clips.get(i))
            .filter(|clip| clip.interval.contains(at));
        out.clip = clip.map(|clip| clip.id);
        if let Some((from, to, progress, transition)) = self.transition_at(at) {
            let (mark, audio_mark) = (out.layers.len(), out.audio.len());
            self.draw_clip(&self.clips[from], at, true, out);
            let (mid, audio_mid) = (out.layers.len(), out.audio.len());
            // Behind-presenter overlays draw once, with the outgoing take (M15).
            let behind = !self.clips[from].take;
            self.draw_clip(&self.clips[to], at, behind, out);
            self.blend(
                transition,
                progress,
                [mark, mid],
                [audio_mark, audio_mid],
                out,
            );
        } else if let Some(clip) = clip {
            let mark = out.layers.len();
            self.draw_clip(clip, at, true, out);
            // A fading clip fades as one image over its backdrop (not through black), except at
            // the timeline's edges, where there is nothing behind it: the backdrop fades too.
            let opacity = clip.interval.alpha(at);
            if opacity < 1.0 {
                let i = clip.interval;
                let edge = (i.start == 0 && at - i.start < i.fade_in)
                    || (i.end == self.end && i.end - at <= i.fade_out);
                let start = mark
                    + usize::from(
                        !edge && matches!(out.layers.get(mark), Some(Layer::Background { .. })),
                    );
                out.group = Some(Group {
                    layers: [start, out.layers.len()],
                    opacity,
                    scale: 1.0,
                    offset: [0.0, 0.0],
                    clip: None,
                });
            }
        }
        // One paint order across kinds (T42.4); equal z keeps overlays, shapes, titles, captions.
        let on_take = clip.is_some_and(|clip| clip.take);
        let mut stacked = 0.0;
        for &(_, kind, index) in &self.paint {
            match kind {
                0 => {
                    let overlay = &self.overlays[index];
                    if !(overlay.behind && on_take) && overlay.interval.contains(at) {
                        self.draw_overlay(overlay, at, out);
                    }
                }
                1 => self.draw_shape(&self.shapes[index], at, out),
                2 => self.draw_title(&self.titles[index], at, &mut stacked, out),
                _ => self.draw_caption(&self.captions[index], at, out),
            }
        }
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
    }

    /// A media overlay above the spine (behind-presenter ones draw with their take).
    fn draw_overlay(&self, overlay: &PreparedOverlay, frame: Frame, out: &mut FramePlan) {
        let opacity = overlay.interval.alpha(frame);
        let local = f64::from(frame - overlay.interval.start);
        let source = overlay.input.frame(frame - overlay.interval.start);
        let look = overlay.rig.look(local, self.size.map(f64::from));
        let look = self.parented(
            look,
            Some(overlay.place.lane),
            frame,
            overlay.visual.picture.destination,
        );
        overlay.visual.draw(
            source.min((overlay.input.frames - 1.0).max(0.0)),
            opacity,
            look,
            true,
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

    fn draw_shape(&self, shape: &PreparedShape, at: Frame, out: &mut FramePlan) {
        if !shape.interval.contains(at) {
            return;
        }
        let canvas = self.size.map(f64::from);
        let local = f64::from(at - shape.interval.start);
        let look = shape.rig.look(local, canvas);
        let fade = shape.interval.alpha(at);
        let bounds = shape.bounds;
        if let Some(emitter) = &shape.particles {
            // Each particle pivots on itself under the emitter's transform (its anchor across
            // the emitter's bounds), then the lane's and the project's.
            for index in 0..emitter.len() {
                let sprite = emitter.sprite(index, local);
                let [w, h] = sprite.size;
                let rect =
                    layout::rect(sprite.center[0] - w / 2.0, sprite.center[1] - h / 2.0, w, h);
                let own = Look {
                    rotation: sprite.rotation,
                    ..Look::default()
                };
                let inside = Rect {
                    x: rect.x - bounds.x,
                    y: rect.y - bounds.y,
                    ..rect
                };
                let look = look::compose(own, look, inside, [bounds.w, bounds.h]);
                out.layers.push(Layer::Shape {
                    item: shape.id,
                    shape: emitter.kind(),
                    bounds: rect,
                    fill: sprite.color,
                    fill2: sprite.color,
                    stroke: sprite.color,
                    stroke_width: 0.0,
                    corner: 0.0,
                    opacity: fade * sprite.opacity,
                    look: self.parented(look, Some(shape.place.lane), at, rect),
                    sweep: 1.0,
                });
            }
            return;
        }
        let look = self.parented(look, Some(shape.place.lane), at, bounds);
        let mut sweep = 1.0;
        if let Some((value, track, track_opacity)) = &shape.progress {
            sweep = value.at(local).clamp(0.0, 1.0);
            out.layers.push(Layer::Shape {
                item: shape.id,
                shape: shape.shape,
                bounds,
                fill: *track,
                fill2: *track,
                stroke: *track,
                stroke_width: if shape.shape == ShapeKind::Ring {
                    shape.stroke_width
                } else {
                    0.0
                },
                corner: shape.corner,
                opacity: fade * track_opacity,
                look,
                sweep: 1.0,
            });
        }
        out.layers.push(Layer::Shape {
            item: shape.id,
            shape: shape.shape,
            bounds,
            fill: shape.fill[0],
            fill2: shape.fill[1],
            stroke: shape.stroke,
            stroke_width: shape.stroke_width,
            corner: shape.corner,
            opacity: fade,
            look,
            sweep,
        });
    }

    fn draw_title(&self, title: &PreparedTitle, at: Frame, stacked: &mut f64, out: &mut FramePlan) {
        if !title.interval.contains(at) {
            return;
        }
        let canvas = self.size.map(f64::from);
        let [w, h] = canvas;
        let (bounds, font, wrap) = match title.text_box {
            // A box (T42.2): the text wraps into it at its style size and shrinks to fit,
            // or starts at the box height and shrinks (`fill`).
            Some(text_box) => {
                let [x, y, bw, bh] = text_box.rect;
                let bounds = layout::rect(x * w, y * h, bw * w, bh * h);
                let size = title.style.as_ref().map_or(0.06, |s| s.size) * h.min(w);
                let font = if text_box.fill { bounds.h } else { size };
                (bounds, font, text_box.wrap)
            }
            // Styled titles size by font (bounds hold one line at 1/0.6 of the font size).
            None => {
                let (height, position) = title.style.as_ref().map_or((h.min(w) * 0.1, 0), |s| {
                    (s.size * h.min(w) / 0.6 * s.line_height / 1.15, s.position)
                });
                let y = match position {
                    1 => (h - height) / 2.0,
                    2 => h * 0.86 - height,
                    3 => h * 0.76,
                    _ => {
                        let y = h * 0.07 + *stacked;
                        *stacked += height * 1.2;
                        y
                    }
                };
                (
                    layout::rect(w * 0.06, y, w * 0.88, height),
                    height * 0.6,
                    true,
                )
            }
        };
        let look = title.rig.look(f64::from(at - title.interval.start), canvas);
        out.layers.push(Layer::Title {
            item: title.id,
            bounds,
            font,
            wrap,
            opacity: title.interval.alpha(at),
            look: self.parented(look, Some(title.place.lane), at, bounds),
        });
    }

    fn draw_caption(&self, caption: &PreparedCaption, at: Frame, out: &mut FramePlan) {
        if !caption.interval.contains(at) {
            return;
        }
        let canvas = self.size.map(f64::from);
        let word = caption.words.iter().rposition(|word| at >= word.start);
        let look = caption
            .rig
            .look(f64::from(at - caption.interval.start), canvas);
        out.layers.push(Layer::Caption {
            item: caption.id,
            word,
            opacity: caption.interval.alpha(at),
            style: caption.style,
            look: self.parented(
                look,
                Some(caption.place.lane),
                at,
                caption_bounds(&caption.style, canvas),
            ),
        });
    }

    /// `look` under its lane's (or, for `None`, the spine's) and the project's transforms
    /// (T42.4): parents scale and rotate about their anchor on the canvas. Keys on lanes and
    /// the project are absolute frames. `dest` is the layer's own rectangle.
    fn parented(&self, look: Look, lane: Option<usize>, at: Frame, dest: Rect) -> Look {
        let parent = match lane {
            Some(index) => self.lane_parents.get(index).and_then(Option::as_ref),
            None => self.spine_parent.as_ref(),
        };
        let canvas = self.size.map(f64::from);
        let mut look = look;
        for rig in [parent, self.project_parent.as_ref()].into_iter().flatten() {
            look = look::compose(look, rig.look(f64::from(at), canvas), dest, canvas);
        }
        look
    }

    /// Prepared title text.
    pub fn title(&self, id: ItemId) -> Option<&str> {
        self.titles
            .iter()
            .find(|title| title.id == id)
            .map(|title| title.text.as_str())
    }

    /// What a title shows at timeline frame `at`: a counter's text there, else its text.
    pub fn title_at(&self, id: ItemId, at: Frame) -> Option<&str> {
        let title = self.titles.iter().find(|title| title.id == id)?;
        Some(
            title
                .runs
                .iter()
                .find(|run| (run.range[0]..run.range[1]).contains(&at))
                .map_or(title.text.as_str(), |run| run.text.as_str()),
        )
    }

    /// A counter title's text runs over its interval (T45.4); empty for a plain title.
    pub fn title_runs(&self, id: ItemId) -> &[TextRun] {
        self.titles
            .iter()
            .find(|title| title.id == id)
            .map_or(&[], |title| title.runs.as_slice())
    }

    /// Resolved text style of a title or caption, when one is set.
    pub fn text_style(&self, id: ItemId) -> Option<&style::TextStyle> {
        self.titles
            .iter()
            .find(|title| title.id == id)
            .and_then(|title| title.style.as_ref())
            .or_else(|| {
                self.captions
                    .iter()
                    .find(|caption| caption.id == id)
                    .and_then(|caption| caption.text.as_ref())
            })
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

    fn draw_clip(&self, clip: &PreparedClip, frame: Frame, behind: bool, out: &mut FramePlan) {
        let Some(input) = &clip.input else { return };
        let local = f64::from(frame - clip.interval.start);
        let source = input.frame(frame - clip.interval.start);
        let opacity = clip.interval.alpha(frame);
        let size = self.size.map(f64::from);
        let mut look = clip.rig.look(local, size);
        let picture_source = source.clamp(0.0, (input.frames - 1.0).max(0.0));
        let view = self.view(frame);
        if let Some(visual) = &clip.visual {
            look = self.parented(look, None, frame, visual.picture.destination);
            let mark = out.layers.len();
            visual.draw(picture_source, 1.0, look, false, out);
            // A whole-picture camera zooms the backdrop with the window (the renderer places
            // the picture from its view); content mode leaves the backdrop alone.
            if !view.mode.zooms_content()
                && let Some(Layer::Background { bounds, .. }) = out.layers.get_mut(mark)
            {
                *bounds = view.canvas(*bounds, size);
            }
            if let Some(Layer::Picture(picture)) = out.layers.last_mut() {
                picture.view = view;
            }
        }
        let (gain, pan) = self.mix(clip.mix.as_ref(), local);
        input.audio(source, TimelineId::Clip(clip.id), opacity * gain, pan, out);
        if clip.take && behind {
            self.draw_overlays(frame, true, true, out);
        }
        if let Some(camera) = &clip.camera {
            let amount = view.amount;
            let bubble = layout::camera(
                size,
                &camera.style,
                camera.screen,
                camera.keys,
                local,
                amount,
                camera.ease,
            );
            out.layers.push(Layer::Camera {
                clip: clip.id,
                media: input.id,
                source_frame: picture_source,
                bounds: bubble.bounds,
                radius: bubble.radius,
                shadow: bubble.shadow,
                background: camera.style.background,
                opacity: bubble.opacity,
                mirror: camera.style.mirror,
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
    }

    fn draw_overlays(&self, frame: Frame, on_take: bool, behind: bool, out: &mut FramePlan) {
        for overlay in self.overlays.iter().filter(|overlay| {
            (overlay.behind && on_take) == behind && overlay.interval.contains(frame)
        }) {
            self.draw_overlay(overlay, frame, out);
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
        // One backdrop (D14): an incoming backdrop identical to the outgoing one is not drawn
        // again, and the outgoing backdrop never moves with its content.
        let outgoing_backdrop = matches!(out.layers.get(a), Some(Layer::Background { .. }));
        let mut shared = false;
        if let (
            Some(Layer::Background {
                bounds: ba,
                fill: fa,
                ..
            }),
            Some(Layer::Background {
                bounds: bb,
                fill: fb,
                ..
            }),
        ) = (out.layers.get(a).copied(), out.layers.get(b).copied())
            && ba == bb
            && fa == fb
        {
            out.layers.remove(b);
            shared = true;
        }
        let end = out.layers.len();
        let content = [a + usize::from(outgoing_backdrop), b];
        let group = |opacity: f64, scale: f64, offset: [f64; 2], clip: Option<[f64; 4]>| Group {
            layers: [b, end],
            opacity,
            scale,
            offset,
            clip,
        };
        match transition.kind {
            TransitionKind::Dissolve => out.group = Some(group(t, 1.0, [0.0, 0.0], None)),
            TransitionKind::Zoom => {
                out.shift = Some(Shift {
                    layers: content,
                    scale: 1.0 + 0.15 * t,
                    offset: [0.0, 0.0],
                });
                out.group = Some(group(t, 1.15 - 0.15 * t, [0.0, 0.0], None));
            }
            // Both sides share a backdrop: dip to it, content only (T36.2).
            TransitionKind::Dip if shared => {
                let keep = content[0];
                if t < 0.5 {
                    out.layers.truncate(b);
                    out.group = Some(Group {
                        layers: [keep, b],
                        opacity: 1.0 - 2.0 * t,
                        scale: 1.0,
                        offset: [0.0, 0.0],
                        clip: None,
                    });
                } else {
                    out.layers.drain(keep..b);
                    out.group = Some(Group {
                        layers: [keep, out.layers.len()],
                        opacity: 2.0 * t - 1.0,
                        scale: 1.0,
                        offset: [0.0, 0.0],
                        clip: None,
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
                out.group = Some(group(1.0, 1.0, [0.0, 0.0], Some(clip)));
            }
            // The kinds below move content only: the shared backdrop stays put (D14, T40.4).
            TransitionKind::Whip => {
                // Cubic in-out: most of the travel happens in the middle third.
                let e = if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                };
                out.shift = Some(Shift {
                    layers: content,
                    scale: 1.0,
                    offset: [dx * e * w, dy * e * h],
                });
                out.group = Some(group(
                    1.0,
                    1.0,
                    [-dx * (1.0 - e) * w, -dy * (1.0 - e) * h],
                    None,
                ));
            }
            TransitionKind::ZoomPunch => {
                // Out: punch to 1.6× while fading; in: settle from 1.6× while appearing.
                let out_t = (2.0 * t).min(1.0);
                let in_t = (2.0 * t - 1.0).max(0.0);
                out.shift = Some(Shift {
                    layers: content,
                    scale: 1.0 + 0.6 * out_t * out_t,
                    offset: [0.0, 0.0],
                });
                let settle = 1.0 - (1.0 - in_t).powi(3);
                out.group = Some(group(
                    smooth(((t - 0.35) / 0.3).clamp(0.0, 1.0)),
                    1.6 - 0.6 * settle,
                    [0.0, 0.0],
                    None,
                ));
            }
            TransitionKind::Swipe => {
                let e = 1.0 - (1.0 - t).powi(3);
                out.shift = Some(Shift {
                    layers: content,
                    scale: 1.0,
                    offset: [dx * e * w / 3.0, dy * e * h / 3.0],
                });
                out.group = Some(group(
                    1.0,
                    1.0,
                    [-dx * (1.0 - e) * w, -dy * (1.0 - e) * h],
                    None,
                ));
            }
            TransitionKind::Push | TransitionKind::Slide => {
                if transition.kind == TransitionKind::Push {
                    out.shift = Some(Shift {
                        layers: content,
                        scale: 1.0,
                        offset: [dx * t * w, dy * t * h],
                    });
                }
                out.group = Some(group(
                    1.0,
                    1.0,
                    [-dx * (1.0 - t) * w, -dy * (1.0 - t) * h],
                    None,
                ));
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

    /// A zoom focus (a point of the spine clip's crop, 0–1; the whole source when uncropped, A5)
    /// as a point of the picture the camera frames at `at`: the same point unless `fill`
    /// trims the crop to the frame's shape. An animated crop (crop.keys) is the picture itself.
    // ponytail: an animated crop whose shape differs from its card is trimmed by the renderer
    // and the focus is not corrected for it; crop.keys writes card-shaped crops.
    fn picture_point(&self, point: [f64; 2], at: Frame) -> [f64; 2] {
        let Some((crop, source)) = self
            .clips
            .iter()
            .find(|clip| clip.interval.contains(at) && !clip.rig.crops())
            .and_then(|clip| Some((clip.crop, clip.visual.as_ref()?.picture.source)))
        else {
            return point;
        };
        [
            (crop.x + point[0] * crop.w - source.x) / source.w,
            (crop.y + point[1] * crop.h - source.y) / source.h,
        ]
        .map(|v| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.5
            }
        })
    }

    /// A content point of the spine picture at `at` as a point of the canvas (normalized).
    fn canvas_point(&self, [x, y]: [f64; 2], at: Frame) -> [f64; 2] {
        let Some(d) = self
            .clips
            .iter()
            .find(|clip| clip.interval.contains(at))
            .and_then(|clip| clip.visual.as_ref())
            .map(|visual| visual.picture.destination)
        else {
            return [x, y];
        };
        let [w, h] = self.size.map(f64::from);
        [(d.x + x * d.w) / w, (d.y + y * d.h) / h].map(|v| v.clamp(0.0, 1.0))
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
        let place = Place {
            z: data.appearance.z,
            lane: rank,
        };
        let fx = AudioFx::new(&audio_scopes);
        if fx != AudioFx::default() {
            self.fx.push((TimelineId::Item(id), fx));
        }
        match &data.kind {
            ItemKind::Title { text } => self.titles.push(PreparedTitle {
                place,
                id,
                interval,
                text: text.clone(),
                rig,
                style: style::text(&[&p.effects, lane, local]),
                text_box: style::text_box(&[&p.effects, lane, local]),
                runs: counter_runs(local, interval.start, interval.end),
            }),
            ItemKind::Shape { shape, rect } => {
                let style = look::shape_style(local);
                let [w, h] = size;
                let bounds = layout::rect(rect.x * w, rect.y * h, rect.w * w, rect.h * h);
                // A ring without a stroke width is an eighth of its diameter thick.
                let stroke_width = if style.3 <= 0.0 && *shape == ShapeKind::Ring {
                    bounds.w.min(bounds.h).abs() / 8.0
                } else {
                    style.3 * w.min(h)
                };
                let progress = style::Resolution::on(&[local], &[], "progress").map(|fx| {
                    (
                        fx.num("value", 0.0),
                        fx.color("track", 0xffffff),
                        fx.number("trackOpacity", 0.25).clamp(0.0, 1.0),
                    )
                });
                let particles = style::Resolution::on(&[local], &[], "particles").map(|fx| {
                    particles::Emitter::new(
                        particles::Preset::parse(fx.text("preset").unwrap_or("confetti")),
                        fx.number("count", 60.0).clamp(1.0, 120.0) as usize,
                        fx.number("seed", 1.0).max(0.0) as u64,
                        fx.number("size", 0.014) * w.min(h),
                        bounds,
                        f64::from(interval.end - interval.start),
                        f64::from(p.fps),
                    )
                });
                self.shapes.push(PreparedShape {
                    place,
                    id,
                    interval,
                    shape: *shape,
                    bounds,
                    fill: [style.0, style.1],
                    stroke: style.2,
                    stroke_width,
                    corner: style.4 * (rect.w * w).min(rect.h * h).abs(),
                    rig,
                    progress,
                    particles,
                });
            }
            ItemKind::Zoom(zoom) => {
                let [low, high] = splice_model::Zoom::AMOUNT;
                if !(low..=high).contains(&zoom.amount)
                    || zoom
                        .focus
                        .is_some_and(|c| c.iter().any(|v| !(0.0..=1.0).contains(v)))
                {
                    return Err(EvalError::Range);
                }
                if !(0.0..=1.0).contains(&zoom.motion_blur) {
                    return Err(EvalError::Range);
                }
                // Content mode frames the picture's content; a whole-picture camera frames the
                // canvas, so its focus is where that content point sits on the canvas.
                let focus = zoom.focus.map(|p| {
                    let point = self.picture_point(p, interval.start);
                    if zoom.mode.zooms_content() {
                        point
                    } else {
                        self.canvas_point(point, interval.start)
                    }
                });
                self.zooms.push(crate::camera::Segment {
                    start: interval.start,
                    end: interval.end,
                    amount: zoom.amount,
                    center: focus
                        .filter(|_| !zoom.follow)
                        .map(|p| crate::View::center_on(p, zoom.amount)),
                    mode: zoom.mode,
                    spring: zoom.spring,
                    follow: zoom.follow,
                    blur: zoom.motion_blur,
                });
            }
            ItemKind::Sound(sound) => {
                let media = media(p, sound.media)?;
                if !sound.volume.is_finite() || sound.volume < 0.0 {
                    return Err(EvalError::Range);
                }
                // Audio unlinked from a take plays its recorded components.
                let tracks = overlay_tracks(media)
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
                let window = backdrop(p, style::window(&[lane, local]))?;
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
                let bounds = overlay.placement.bounds(size);
                let visual = if media.kind == MediaKind::Audio {
                    blank_visual(media.id, TimelineId::Item(id), bounds, None)
                } else {
                    visual(media, TimelineId::Item(id), bounds, overlay.framing, window)?
                };
                if rig.cutout() {
                    self.cutouts.push((TimelineId::Item(id), media.id));
                }
                self.overlays.push(PreparedOverlay {
                    place,
                    interval,
                    input,
                    visual,
                    behind: overlay.behind_presenter,
                    rig,
                    mix: Mix::new(&audio_scopes, &shifts),
                });
            }
            ItemKind::Caption { words, edits, text } => {
                if let Some(style) = style::caption(&[&p.effects, lane, local]) {
                    let free = text
                        .as_deref()
                        .map(|text| (text, interval.start, interval.end));
                    let words =
                        caption_tokens(p, timing.anchor, words, edits, free, style.uppercase)?;
                    if !words.is_empty() {
                        self.captions.push(PreparedCaption {
                            place,
                            id,
                            interval,
                            style,
                            words,
                            rig,
                            text: style::text(&[&p.effects, lane, local]),
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
        click_sound: false,
        visual: None,
        camera: None,
        keys: None,
        hidden_keys: data.hidden_keys.clone(),
        crop: data
            .framing
            .crop
            .unwrap_or(layout::rect(0.0, 0.0, 1.0, 1.0)),
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
        let window = backdrop(p, style::window(&scopes))?;
        let bounds = layout::rect(0.0, 0.0, w, h);
        let mut visual = if screen {
            visual(media, TimelineId::Clip(id), bounds, data.framing, window)?
        } else {
            // A camera-only take still has a backdrop even without a screen size.
            blank_visual(media.id, TimelineId::Clip(id), bounds, window)
        };
        if clip.take && enabled.pointer == Some(true) {
            visual.picture.cursor = style::cursor(&scopes);
        }
        clip.click_sound = clip.take && style::click_sound(&scopes);
        clip.visual = Some(visual);
    }
    if clip.take {
        let keys = enabled.keys == Some(true);
        if keys {
            clip.keys = style::keys(&scopes);
        }
        if enabled.camera == Some(true) {
            clip.camera = Some(Camera {
                style: style::camera(&scopes, &shifts),
                screen,
                keys,
                ease: (0.3 * f64::from(p.fps)).round().max(1.0),
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

/// A window whose image backdrop names a still image of the project (T46.3).
fn backdrop(
    project: &Project,
    window: Option<style::Window>,
) -> Result<Option<style::Window>, EvalError> {
    if let Some(Background::Image(id, _)) = window.map(|w| w.background)
        && media(project, id)?.kind != MediaKind::Image
    {
        return Err(EvalError::Media(id));
    }
    Ok(window)
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
            view: crate::View::IDENTITY,
            opacity: 1.0,
            window,
            cursor: None,
            look: Look::default(),
        },
    }
}

/// Speech on the timeline, for ducking: transcript words, or else clips with a microphone.
/// Spans closer than 12 frames merge.
fn speech(p: &Project, clips: &[PreparedClip]) -> Result<Vec<(Frame, Frame)>, EvalError> {
    let mut spans = Vec::new();
    for &slot in p.clips.order() {
        timing::visible_words(p, slot, &mut |_, span| spans.push((span.start, span.end)))?;
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
/// A free-text cue (`free`: its text and timeline `[start, end)`) is one token with the
/// [`FREE_TEXT`] ID.
pub fn caption_tokens(
    p: &Project,
    anchor: Anchor,
    ids: &[WordId],
    edits: &[CaptionEdit],
    free: Option<(&str, Frame, Frame)>,
    uppercase: bool,
) -> Result<Vec<CaptionWord>, EvalError> {
    if let Some((text, start, end)) = free {
        return Ok(vec![CaptionWord {
            id: FREE_TEXT,
            text: if uppercase {
                text.to_uppercase()
            } else {
                text.into()
            },
            emoji: None,
            emph: false,
            start,
            end,
        }]);
    }
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
    let near = match anchor {
        Anchor::ClipSource { clip, .. } => Some(clip),
        _ => None,
    };
    let mut words = Vec::new();
    for &id in ids {
        let Some(span) = timing::word_span(p, id, hash, near)? else {
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

/// A counter title's text per frame of `[start, end)`, merged into runs of equal text (T45.4).
/// Keys of `counter.value` are relative to the title's start.
fn counter_runs(local: &[Effect], start: Frame, end: Frame) -> Vec<TextRun> {
    let Some(counter) = style::Resolution::on(&[local], &[], "counter") else {
        return Vec::new();
    };
    let value = counter.num("value", 0.0);
    let format = counter.text("format").unwrap_or("{n}");
    let decimals = counter.number("decimals", 0.0).clamp(0.0, 4.0) as u32;
    let group = counter.boolean("group", true);
    let mut runs: Vec<TextRun> = Vec::new();
    for at in start..end {
        let text = counter_text(value.at(f64::from(at - start)), format, decimals, group);
        match runs.last_mut() {
            Some(last) if last.text == text => last.range[1] = at + 1,
            _ => runs.push(TextRun {
                range: [at, at + 1],
                text,
            }),
        }
    }
    runs
}

/// Smoothstep on [0, 1].
fn smooth(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

/// A caption cue's text rectangle in canvas pixels: the safe-zone band the renderer lays the
/// cue out in (splice_av lowering uses the same formula).
fn caption_bounds(style: &CaptionStyle, [w, h]: [f64; 2]) -> Rect {
    let [top, bottom, left, right] = style.safe;
    let font = (w * style.size).min(h * 0.15);
    let height = font * 2.5;
    let y = (h * style.position - height / 2.0)
        .clamp(h * top, (h * (1.0 - bottom) - height).max(h * top));
    layout::rect(w * left, y, w * (1.0 - left - right), height)
}
