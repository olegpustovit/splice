use splice_model::{
    Access, Aspect, CameraMode, Duck, Ease, Edge, EffectId, EffectParam, Fit, Frame, Key, LaneId,
    LaneKind, MediaId, Placement, Rect, ShapeKind, Spring, TimelineId, WordId, Zoom,
};

/// What smart cuts measure pauses against (D15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Basis {
    /// Transcript words when the timeline has any, else recorded activity.
    #[default]
    Auto,
    /// Transcript words only.
    Transcript,
    /// Recorded input activity of screen takes only.
    Activity,
}

/// An edit command with its validated wire arguments.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Split at Playhead.
    ItemSplit {
        /// Item to split.
        item: Target,
        /// Split frame.
        at: FrameArg,
    },
    /// Trim.
    ItemTrim {
        /// Item to trim.
        item: Target,
        /// Edge to adjust.
        edge: TrimEdge,
        /// Absolute frame or relative offset.
        position: Position,
        /// Shift subsequent spine clips.
        ripple: bool,
    },
    /// Move.
    ItemMove {
        /// Selected items in input order.
        items: Vec<Target>,
        /// Destination of the first item or the shared delta.
        position: Position,
        /// Move attached overlays with a spine clip.
        carry_overlays: bool,
        /// Reorder a spine clip when it collides.
        ripple: bool,
    },
    /// Delete.
    ItemDelete {
        /// Selected items.
        items: Vec<Target>,
        /// Close the gap after a deleted clip.
        ripple: bool,
        /// Keep overlays pinned to a gap.
        keep_overlays: bool,
        /// What happens to items anchored to deleted clips (D21).
        connected: Option<Connected>,
    },
    /// Duplicate.
    ItemDuplicate {
        /// Selected items.
        items: Vec<Target>,
        /// Explicit paste position.
        at: Option<FrameArg>,
    },
    /// Set Properties.
    ItemSet {
        /// Selected items.
        items: Vec<Target>,
        /// Optional name update. Null clears the name.
        name: Option<Option<String>>,
        /// Optional swatch update. Null clears the swatch.
        color: Option<Option<String>>,
        /// New fade-in duration.
        fade_in: Option<Frame>,
        /// New fade-out duration.
        fade_out: Option<Frame>,
    },
    /// Speed.
    ClipSpeed {
        /// Selected clips.
        items: Vec<Target>,
        /// Playback rate multiplier.
        speed: f64,
        /// Shift subsequent clips.
        ripple: bool,
    },
    /// Insert Media.
    ClipInsert {
        /// Media ID or file to import.
        source: InsertSource,
        /// Destination. Omit to append.
        at: Option<FrameArg>,
        /// Shift clips at or after the destination.
        ripple: bool,
        /// Whether to create a clip after registering the file.
        insert: bool,
        /// Audio lane for a sound item.
        lane: Option<Target>,
    },
    /// Lift to Overlay.
    ClipLift {
        /// Selected clips.
        items: Vec<Target>,
        /// Explicit overlay lane.
        lane: Option<Target>,
    },
    /// Replace Spine Clip.
    OverlayReplaceSpine {
        /// Media overlay to move to the spine.
        item: Target,
        /// Clip to replace. Omit to use the anchor or the clip under the overlay.
        target: Option<Target>,
    },
    /// Add as Overlay.
    OverlayAdd {
        /// Kind-specific parameters.
        content: OverlayContent,
        /// Start position.
        at: Option<FrameArg>,
        /// Explicit anchor when supplied.
        anchor: Option<AnchorArg>,
        /// Destination lane.
        lane: Option<Target>,
        /// Duration or the kind default.
        length: Option<Frame>,
    },
    /// Add Title.
    TitleAdd {
        /// Start position.
        at: FrameArg,
        /// Duration.
        length: Frame,
        /// Visible text.
        text: Option<String>,
        /// Explicit graphics lane.
        lane: Option<Target>,
        /// Inline box, style and animation (T42.2).
        look: Option<Box<TitleLook>>,
    },
    /// Edit Title.
    TitleSet {
        /// Title to edit.
        item: Target,
        /// New visible text.
        text: Option<String>,
        /// Inline box, style and animation (T42.2).
        look: Option<Box<TitleLook>>,
    },
    /// Add Zoom: a camera segment (D12).
    ZoomAdd {
        /// Start position.
        at: FrameArg,
        /// Duration.
        length: Frame,
        /// The segment's framing.
        zoom: Zoom,
        /// Camera lane (default: the first, created if missing).
        lane: Option<Target>,
    },
    /// Remove Zoom: deletes one camera segment.
    ZoomRemove {
        /// Zoom item to remove.
        zoom: Target,
    },
    /// Edit Zoom.
    ZoomEdit {
        /// Zoom item to edit.
        zoom: Target,
        /// New magnification.
        amount: Option<f64>,
        /// New focus point; null keeps the current framing.
        focus: Option<Option<[f64; 2]>>,
        /// New mode.
        mode: Option<CameraMode>,
        /// New spring.
        spring: Option<Spring>,
    },
    /// Pin / Unpin.
    ItemAnchor {
        /// Item to anchor.
        item: Target,
        /// How to derive or store the anchor.
        mode: AnchorMode,
    },
    /// Move to Lane.
    ItemLane {
        /// Items to move.
        items: Vec<Target>,
        /// Explicit lane or direction in the lane stack.
        destination: LaneMove,
    },
    /// Overlay Settings.
    OverlaySet {
        /// Media overlay.
        item: Target,
        /// New destination in the canvas.
        placement: Option<Placement>,
        /// Place behind the presenter.
        behind_presenter: Option<bool>,
        /// Mute media audio.
        muted: Option<bool>,
    },
    /// New Lane.
    LaneCreate {
        /// Lane category.
        kind: LaneKind,
        /// Explicit display name.
        name: Option<String>,
    },
    /// Rename Lane.
    LaneRename {
        /// Lane to rename.
        lane: Target,
        /// New display name.
        name: String,
    },
    /// Delete Lane.
    LaneDelete {
        /// Lane to remove.
        lane: Target,
    },
    /// Show / Hide Component.
    ComponentSet {
        /// Recorded clip.
        clip: Target,
        /// Recorded component to switch.
        component: Component,
        /// New enable state.
        enabled: bool,
    },
    /// Volume.
    AudioSet {
        /// Recorded clip and its microphone or system component.
        clip: Option<(Target, Component)>,
        /// Sound items.
        items: Vec<Target>,
        /// New gain.
        volume: Option<f64>,
        /// Ducking for sound items. Null removes it.
        duck: Option<Option<Duck>>,
        /// LUFS target for sound items.
        normalize: Option<f64>,
        /// Write the ducking as visible volume keys instead of keeping it computed.
        bake: bool,
    },
    /// Cursor Settings.
    CursorSet {
        /// Recorded clip.
        clip: Target,
        /// Cursor scale.
        size: Option<f64>,
        /// Draw click rings.
        click_rings: Option<bool>,
    },
    /// Keystroke Chip Settings.
    KeysSet {
        /// Recorded clip.
        clip: Target,
        /// Keystroke chip scale.
        size: Option<f64>,
    },
    /// Hide Keystrokes.
    KeysHide {
        /// Recorded clip.
        clip: Target,
        /// Source keystroke event indices.
        events: Vec<u32>,
        /// Hide or show the events.
        hidden: bool,
    },
    /// Add Marker.
    MarkerAdd {
        /// Marker position.
        at: FrameArg,
        /// Explicit marker name.
        name: Option<String>,
    },
    /// Edit Marker.
    MarkerSet {
        /// Marker to edit.
        marker: Target,
        /// New marker name.
        name: Option<String>,
        /// New marker position.
        at: Option<FrameArg>,
    },
    /// Output Shape.
    ProjectSet {
        /// New output aspect.
        aspect: Option<Aspect>,
        /// New agent access level.
        access: Option<Access>,
    },
    /// Crop.
    ClipCrop {
        /// Clips or media overlays to crop.
        items: Vec<Target>,
        /// Crop change. Null resets the crop.
        rect: Option<Option<Rect>>,
        /// New fit mode.
        fit: Option<Fit>,
        /// Reframe: crop to this width/height ratio (`None` inside means the canvas shape).
        aspect: Option<Option<f64>>,
        /// Source point the reframe keeps in frame.
        focus: Option<[f64; 2]>,
    },
    /// Delete Range.
    CutRange {
        /// Inclusive start.
        from: FrameArg,
        /// Exclusive end.
        to: FrameArg,
        /// Close the cut range.
        ripple: bool,
        /// What happens to items anchored to the cut frames (D21).
        connected: Option<Connected>,
    },
    /// Delete Words.
    CutWords {
        /// Words to cut.
        words: Vec<WordId>,
        /// Expected transcript version.
        transcript_hash: Option<u64>,
        /// What happens to items anchored to the cut frames (D21).
        connected: Option<Connected>,
    },
    /// Add Effect.
    EffectAdd {
        /// Effect owner.
        target: Target,
        /// Effect type.
        kind: String,
        /// Explicit parameter values.
        params: Vec<EffectParam>,
        /// Set an enable override.
        enabled: Option<bool>,
    },
    /// Set Effect Parameters.
    EffectSet {
        /// Existing effect or owner and type.
        target: EffectSelection,
        /// Parameters to override.
        params: Vec<EffectParam>,
        /// Set an enable override.
        enabled: Option<bool>,
        /// Parameters whose overrides are removed.
        reset: Vec<String>,
    },
    /// Remove Effect.
    EffectRemove {
        /// Explicit effect IDs.
        effects: Vec<Target>,
        /// Remove effects from this owner.
        target: Option<Target>,
        /// Remove only this effect type.
        kind: Option<String>,
    },
    /// Paste Effects.
    EffectPaste {
        /// Owner to copy from.
        from: Target,
        /// Destination owners.
        to: Vec<Target>,
        /// Replace existing effects on each destination.
        replace: bool,
    },
    /// Promote Effect.
    EffectPromote {
        /// Effect to promote.
        effect: Target,
        /// Promote to the project instead of the lane.
        to_project: bool,
        /// Remove redundant overrides on sibling items.
        clear_overrides: bool,
    },
    /// Generate Captions.
    CaptionsGenerate {
        /// Explicit clips to caption.
        clips: Option<Vec<Target>>,
        /// Maximum words in a cue.
        max_words: Frame,
        /// Include filler words.
        fillers: bool,
        /// Explicit captions lane.
        lane: Option<Target>,
        /// Caption style preset.
        style: Option<String>,
    },
    /// Edit Caption.
    CaptionSet {
        /// Caption cue.
        item: Target,
        /// Replacement cue text.
        text: Option<String>,
        /// Changes to individual displayed words.
        words: Vec<CaptionWordPatch>,
    },
    /// Split Caption.
    CaptionSplit {
        /// Caption cue.
        item: Target,
        /// First word in the new cue.
        word: WordId,
    },
    /// Merge Captions.
    CaptionMerge {
        /// Captions to merge.
        items: Vec<Target>,
    },
    /// Join Clips.
    ClipJoin {
        /// Adjacent spine clips from one source, in order.
        items: Vec<Target>,
    },
    /// Ungroup.
    ClipUngroup {
        /// A joined clip.
        item: Target,
    },
    /// Restore Cuts.
    ClipRestore {
        /// Clips whose cut material to bring back; empty restores every seam.
        items: Vec<Target>,
    },
    /// Reorder Lane.
    LaneReorder {
        /// Lane to move.
        lane: Target,
        /// New stack index, 0 at the top.
        to: usize,
    },
    /// Animated Crop.
    CropKeys {
        /// Clip or media overlay.
        item: Target,
        /// Owner-relative keys: frame, crop x/y/w/h normalized to the source, ease.
        keys: Vec<(Frame, [f64; 4], Ease)>,
    },
    /// Add Transition.
    TransitionAdd {
        /// Incoming spine clip.
        item: Target,
        /// Transition parameters.
        params: Vec<EffectParam>,
    },
    /// Zoom Settings.
    /// Add Mask.
    MaskAdd {
        /// Clip or media overlay.
        item: Target,
        /// Mask parameters.
        params: Vec<EffectParam>,
    },
    /// Edit Mask.
    MaskSet {
        /// Mask effect.
        mask: Target,
        /// Mask parameters.
        params: Vec<EffectParam>,
        /// Turn the mask on or off.
        enabled: Option<bool>,
    },
    /// Translate Captions.
    CaptionsTranslate {
        /// Captions lane to translate.
        lane: Target,
        /// BCP 47 tag of the translation.
        language: String,
        /// One translated text per cue, in cue order.
        texts: Vec<String>,
    },
    /// Import SRT / VTT.
    CaptionsImport {
        /// Destination captions lane; a new one when omitted.
        lane: Option<Target>,
        /// SRT or WebVTT text.
        text: String,
    },
    /// Link Audio.
    AudioLink {
        /// Detached sound item.
        item: Target,
    },
    /// Unlink Audio.
    AudioUnlink {
        /// Spine clip whose audio becomes a separate sound item.
        item: Target,
        /// Destination audio lane.
        lane: Option<Target>,
    },
    /// Move into Sync.
    AudioSync {
        /// Sound item or media overlay to align.
        item: Target,
        /// Spine clip it belongs to.
        to: Target,
        /// Source frames of `item` that line up with the first source frame of `to`.
        offset: Frame,
    },
    /// Style.
    StyleSet {
        /// Brand kit parameters.
        params: Vec<EffectParam>,
    },
    /// Cut Pauses.
    CutPauses {
        /// Pauses at least this long are cut (frames).
        min: Option<Frame>,
        /// Silence kept on each side of a cut (frames).
        keep: Option<Frame>,
        /// What a pause is measured against.
        basis: Basis,
        /// Busy source ranges of takes, for the activity basis.
        activity: Vec<(MediaId, Frame, Frame)>,
    },
    /// Tighten Pace.
    IntentPace {
        /// Longest pause to keep (frames).
        max_pause: Option<Frame>,
        /// What a pause is measured against.
        basis: Basis,
        /// Busy source ranges of takes, for the activity basis.
        activity: Vec<(MediaId, Frame, Frame)>,
    },
    /// Fit to Duration.
    IntentFitDuration {
        /// Target length in seconds.
        seconds: f64,
        /// What a pause is measured against.
        basis: Basis,
        /// Busy source ranges of takes, for the activity basis.
        activity: Vec<(MediaId, Frame, Frame)>,
    },
    /// Keep Shots.
    IntentKeepShots {
        /// Timeline ranges to keep, half-open; everything else on the spine is cut.
        ranges: Vec<(Frame, Frame)>,
    },
    /// Auto Zoom.
    IntentAutoZoom {
        /// Screen take clip.
        item: Target,
        /// Clicks as (source frame, x, y), x and y normalized to the source.
        clicks: Vec<(Frame, f64, f64)>,
        /// Typing as source frame ranges: extends a zoom, never moves it.
        typing: Vec<(Frame, Frame)>,
        /// Magnification (default 1.5).
        amount: Option<f64>,
    },
    /// Steps (Splice extension): a screen demo trimmed to its steps.
    IntentSteps {
        /// Screen take clip.
        item: Target,
        /// Frames kept before each step starts.
        lead: Option<Frame>,
        /// Frames kept after a step's last activity.
        settle: Option<Frame>,
        /// Longer quiet stretches inside a step are cut (frames).
        max_gap: Option<Frame>,
        /// Marker name prefix that restarts the step before it.
        retake_prefix: Option<String>,
        /// Busy source frame ranges of the take.
        activity: Vec<(Frame, Frame)>,
    },
    /// Speed Up Typing (Splice extension).
    IntentSpeedTyping {
        /// Screen take clip.
        item: Target,
        /// Rate while typing.
        factor: f64,
        /// Shorter typing runs keep their speed (frames).
        min_run: Option<Frame>,
        /// Typing as source frame ranges.
        typing: Vec<(Frame, Frame)>,
    },
    /// Add Caption (Splice extension): a free-text cue.
    CaptionAdd {
        /// Cue text.
        text: String,
        /// First timeline frame.
        from: Frame,
        /// Timeline frame after the cue.
        to: Frame,
        /// Destination captions lane; the first one (or a new one) when omitted.
        lane: Option<Target>,
    },
    /// Roll Edit (Splice extension).
    ItemRoll {
        /// Spine clip whose out-edge moves with the next clip's in-edge.
        item: Target,
        /// Signed frames.
        by: Frame,
    },
    /// Slip (Splice extension).
    ItemSlip {
        /// Clip or media item.
        item: Target,
        /// Signed source frames.
        by: Frame,
    },
    /// Slide (Splice extension).
    ItemSlide {
        /// Spine clip moved between its neighbors.
        item: Target,
        /// Signed frames.
        by: Frame,
    },
    /// Freeze Frame (Splice extension).
    ClipFreeze {
        /// Spine clip.
        item: Target,
        /// Timeline frame to hold.
        at: FrameArg,
        /// Hold length in frames.
        length: Frame,
    },
    /// Compound Clip (Splice extension).
    ClipCompound {
        /// Adjacent spine clips.
        items: Vec<Target>,
    },
    /// Break Apart Compound (Splice extension).
    ClipUncompound {
        /// Compound clip.
        item: Target,
    },
    /// Edit Transition (Splice extension).
    TransitionSet {
        /// Incoming spine clip.
        item: Target,
        /// Transition parameters.
        params: Vec<EffectParam>,
    },
    /// Remove Transition (Splice extension).
    TransitionRemove {
        /// Incoming spine clip.
        item: Target,
    },
    /// Set Parameter (Splice extension).
    ParamSet {
        /// Clip, item, lane, effect owner or the project.
        target: Target,
        /// `type.param`, e.g. `transform.scale`.
        path: String,
        /// Constant value.
        value: f64,
    },
    /// Keyframe Parameter (Splice extension).
    ParamKeys {
        /// Clip, item, lane, effect owner or the project.
        target: Target,
        /// `type.param`, e.g. `transform.scale`.
        path: String,
        /// Keys relative to the owner's start.
        keys: Vec<Key>,
        /// Merge into existing keys instead of replacing them.
        merge: bool,
    },
    /// Track Mask (Splice extension).
    MaskTrack {
        /// Mask effect.
        mask: Target,
        /// Mask center keys (owner frame, x, y), normalized to the destination.
        keys: Vec<(Frame, f64, f64)>,
    },
    /// Add Shape (Splice extension).
    GeneratorAdd {
        /// What to draw.
        shape: ShapeKind,
        /// Start.
        at: FrameArg,
        /// Duration.
        length: Option<Frame>,
        /// Normalized canvas bounds.
        rect: Option<Rect>,
        /// Graphics lane.
        lane: Option<Target>,
        /// Shape style parameters.
        params: Vec<EffectParam>,
    },
    /// Add Template (Splice extension).
    TemplateAdd {
        /// Template name.
        name: String,
        /// Start.
        at: FrameArg,
        /// Duration.
        length: Option<Frame>,
        /// Text fields (title, subtitle, …).
        fields: Vec<(String, String)>,
    },
    /// Edit Template (Splice extension).
    TemplateSet {
        /// Any item of the template.
        item: Target,
        /// Text fields to change.
        fields: Vec<(String, String)>,
    },
    /// Declare Pointer (Splice extension, T39.4): positions and presses the recorder never saw.
    TakePointer {
        /// Spine clip of a take.
        clip: Target,
        /// (timeline frame, x, y, press).
        marks: Vec<(Frame, f64, f64, bool)>,
    },
    /// Relink Media (Splice extension).
    MediaRelink {
        /// Media descriptor.
        media: Target,
        /// New file path.
        path: String,
        /// Probed length of the new file, checked against the descriptor unless `force`.
        frames: Option<Frame>,
        /// Accept a different length.
        force: bool,
    },
    /// Consolidate Media (Splice extension).
    MediaConsolidate {
        /// Descriptors to copy into the package; all external media when empty.
        media: Vec<Target>,
    },
    /// Set Transcript (a Splice extension).
    TranscriptSet {
        /// Source media.
        media: Target,
        /// Words as (text, from, to) in source frames.
        words: Vec<(String, Frame, Frame)>,
        /// BCP 47 language tag.
        language: String,
    },
    /// Paint order (T42.4): higher z draws in front, across every kind.
    ItemOrder {
        /// Item to move in the stack.
        item: Target,
        /// Where it goes.
        to: Order,
    },
    /// Motion preset (T42.5): writes ordinary transform keys.
    MotionPreset {
        /// Clips or items to animate.
        items: Vec<Target>,
        /// What moves, with its length and strength.
        motion: Motion,
        /// First frame, relative to each item's start.
        at: Frame,
        /// Whether `length` was given (loops otherwise fill the item).
        sized: bool,
    },
}

/// A typed ID or a reference to a preceding operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Clip, overlay, or marker.
    Timeline(TimelineId),
    /// Lane.
    Lane(LaneId),
    /// Media source.
    Media(MediaId),
    /// An effect instance.
    Effect(EffectId),
    /// The project-level effect scope.
    Project,
    /// Result of an earlier operation.
    Ref(Reference),
}

/// A projection from a batch-local operation result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// Name including its dollar sign.
    pub name: String,
    /// Which part of the referenced operation to use.
    pub part: RefPart,
}

/// Position in a referenced operation's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefPart {
    /// The operation's primary item.
    Primary,
    /// The first created item, as named by `.right` or `.new`.
    FirstCreated,
}

/// An absolute sequence frame or a frame resolved at application time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameArg {
    /// Absolute sequence frame.
    Frame(Frame),
    /// Relative to an item or a previous operation's result.
    Item {
        /// Item, clip, or batch-local reference.
        item: Target,
        /// Start or end of the interval.
        edge: Edge,
        /// Signed timeline frame offset.
        offset: Frame,
    },
    /// Relative to a named marker.
    Marker {
        /// Marker ID or batch-local reference.
        marker: Target,
        /// Signed sequence frame offset.
        offset: Frame,
    },
    /// Relative to a word in its transcript.
    Word {
        /// Word ID.
        word: WordId,
        /// Optional expected transcript version.
        transcript_hash: Option<u64>,
        /// Start or end of the word.
        edge: Edge,
        /// Signed sequence frame offset.
        offset: Frame,
    },
}

/// Explicit item anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorArg {
    /// Pin to a timeline position. Omit the frame to use the operation default.
    Pinned(Option<FrameArg>),
    /// Follow a source frame on the named clip.
    ClipSource {
        /// Clip or batch-local reference.
        clip: Target,
        /// Frame in the clip's source media.
        source: Frame,
    },
    /// Anchor form supplied as a sequence position.
    At(FrameArg),
}

/// A requested absolute position or relative move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Position {
    /// Absolute destination.
    To(FrameArg),
    /// Frame offset.
    Delta(Frame),
}

/// What a cut does with items anchored to the clip frames it removes (D21). Unset: a partial cut
/// re-parents and a whole deleted parent cascades.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connected {
    /// Move them to the nearest surviving clip at the cut point.
    Reparent,
    /// Delete them with the frames.
    Delete,
    /// Keep the part of each that runs past the cut; delete the ones the cut covers.
    Trim,
}

/// Which edge a trim moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimEdge {
    /// In edge.
    In,
    /// Out edge.
    Out,
}

/// Media input for clip insertion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsertSource {
    /// Already in the project's media table.
    Media(Target),
    /// File to import through the native backend.
    Path(String),
}

/// Parameters specific to the content of a new overlay.
#[derive(Clone, Debug, PartialEq)]
pub enum OverlayContent {
    /// Visible title.
    Title(Option<String>),
    /// Camera segment.
    Zoom(Zoom),
    /// Media overlay.
    Media {
        /// The referenced media descriptor.
        media: Target,
        /// First frame of the source.
        source: Frame,
        /// Destination in the canvas.
        placement: Placement,
        /// Keep the presenter in front.
        behind_presenter: bool,
        /// Mute media audio.
        muted: bool,
        /// Normalized source crop.
        crop: Option<Rect>,
        /// Scale mode.
        fit: Fit,
    },
    /// Generated shape.
    Shape {
        /// What to draw.
        shape: ShapeKind,
        /// Normalized canvas bounds.
        rect: Rect,
    },
}

/// Action for the current item's anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorMode {
    /// Pin at the current frame or re-attach at the current frame according to the flag.
    Current(bool),
    /// Apply an explicit anchor.
    Explicit(AnchorArg),
}

/// Where to move an overlay in the lane stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneMove {
    /// Explicit destination.
    To(Target),
    /// One compatible lane up.
    Up,
    /// One compatible lane down.
    Down,
}

/// A component of a recorded take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    /// Screen video.
    Screen,
    /// Camera video.
    Camera,
    /// Microphone audio.
    Mic,
    /// System audio.
    System,
    /// Cursor.
    Pointer,
    /// Keystrokes.
    Keys,
}

/// Identifies an existing effect or the requested effect type at an owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectSelection {
    /// Explicit effect ID.
    Id(Target),
    /// Owner and type.
    Owner {
        /// Effect owner.
        target: Target,
        /// Effect type name.
        kind: String,
    },
}

/// A title's inline look (D16, T42.2); each part compiles to an ordinary effect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleLook {
    /// `layout` parameters: the text box, when `box` was passed.
    pub text_box: Option<Vec<EffectParam>>,
    /// `text` parameters (`style`).
    pub style: Vec<EffectParam>,
    /// `animate` parameters (`in`): how units come in.
    pub enter: Vec<EffectParam>,
    /// `out`: transform keys over the title's last frames.
    pub exit: Option<Motion>,
    /// `loop`: transform keys repeating until the exit.
    pub repeat: Option<Motion>,
}

/// A motion preset with its timing; compiles to transform keys (T42.5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// What moves.
    pub preset: MotionPreset,
    /// Frames the move lasts, or one period of a loop.
    pub length: Frame,
    /// Strength 0–1.
    pub intensity: f64,
}

/// Motion presets (T42.5); `out` takes the exit presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionPreset {
    /// Scale up from small on a bouncy spring.
    BounceIn,
    /// Quick decaying side-to-side shake.
    Shake,
    /// Gentle repeating scale pulse.
    Pulse,
    /// Slow up-and-down drift.
    Float,
    /// Small repeating rotation.
    Wiggle,
    /// Scale up past the size and back.
    Pop,
    /// Exit: fade out.
    Fade,
    /// Exit: drop down while fading.
    Sink,
    /// Exit: shrink away while fading.
    Shrink,
    /// Exit: slide off to the left while fading.
    Slide,
}

impl MotionPreset {
    /// `motion.preset` names.
    pub const MOTION: [(Self, &'static str); 6] = [
        (Self::BounceIn, "bounceIn"),
        (Self::Shake, "shake"),
        (Self::Pulse, "pulse"),
        (Self::Float, "float"),
        (Self::Wiggle, "wiggle"),
        (Self::Pop, "pop"),
    ];
    /// `title.add {out}` names.
    pub const EXIT: [(Self, &'static str); 4] = [
        (Self::Fade, "fade"),
        (Self::Sink, "sink"),
        (Self::Shrink, "shrink"),
        (Self::Slide, "slide"),
    ];
    /// `title.add {loop}` names.
    pub const LOOP: [(Self, &'static str); 4] = [
        (Self::Pulse, "pulse"),
        (Self::Float, "float"),
        (Self::Wiggle, "wiggle"),
        (Self::Shake, "shake"),
    ];
}

/// Where `item.order` puts an item in the paint order (higher z = in front).
#[derive(Clone, Debug, PartialEq)]
pub enum Order {
    /// In front of every other item.
    Front,
    /// Behind every other item.
    Back,
    /// Just in front of this item.
    Above(Target),
    /// Just behind this item.
    Below(Target),
    /// This exact z.
    Z(i32),
}

/// Edit or reset a caption word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionWordPatch {
    /// Word to change.
    pub word: WordId,
    /// Overrides to update. Null resets the word.
    pub edit: Option<CaptionWordEdit>,
}

/// Individual display updates for a caption word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionWordEdit {
    /// Updated display text. Null clears the override.
    pub text: Option<Option<String>>,
    /// Updated emoji. Null clears the override.
    pub emoji: Option<Option<String>>,
    /// Updated emphasis. Null clears the override.
    pub emph: Option<Option<bool>>,
    /// Updated visibility. Null clears the override.
    pub hidden: Option<Option<bool>>,
}
