use splice_model::{
    Access, Aspect, Duck, Edge, EffectId, EffectParam, Fit, Frame, LaneId, LaneKind, MediaId,
    Placement, Rect, TimelineId, WordId,
};

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
    },
    /// Edit Title.
    TitleSet {
        /// Title to edit.
        item: Target,
        /// New visible text.
        text: String,
    },
    /// Add Scale Animation.
    MotionAdd {
        /// Start position.
        at: FrameArg,
        /// Duration.
        length: Frame,
        /// Starting scale.
        from: f64,
        /// Ending scale.
        to: f64,
        /// Canvas point (0…1) the scale holds still.
        focus: Option<[f64; 2]>,
        /// Ease in and out.
        ease: bool,
    },
    /// Edit Scale Keyframes.
    MotionSet {
        /// Motion item to edit.
        item: Target,
        /// New starting scale.
        from: Option<f64>,
        /// New ending scale.
        to: Option<f64>,
        /// New focus. Null resets to the center.
        focus: Option<Option<[f64; 2]>>,
        /// New easing.
        ease: Option<bool>,
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
    },
    /// Delete Words.
    CutWords {
        /// Words to cut.
        words: Vec<WordId>,
        /// Expected transcript version.
        transcript_hash: Option<u64>,
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
    ClipJoin,
    /// Ungroup.
    ClipUngroup,
    /// Restore Cuts.
    ClipRestore,
    /// Reorder Lane.
    LaneReorder,
    /// Animated Crop.
    CropKeys,
    /// Add Transition.
    TransitionAdd,
    /// Zoom Settings.
    ZoomSet,
    /// Add Mask.
    MaskAdd,
    /// Edit Mask.
    MaskSet,
    /// Translate Captions.
    CaptionsTranslate,
    /// Import SRT / VTT.
    CaptionsImport,
    /// Link Audio.
    AudioLink,
    /// Unlink Audio.
    AudioUnlink,
    /// Move into Sync.
    AudioSync,
    /// Style.
    StyleSet,
    /// Cut Pauses.
    CutPauses,
    /// Tighten Pace.
    IntentPace,
    /// Fit to Duration.
    IntentFitDuration,
    /// Keep Shots.
    IntentKeepShots,
    /// Auto Zoom.
    IntentAutoZoom,
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
    /// Scale motion.
    Motion {
        /// Starting scale.
        from: f64,
        /// Ending scale.
        to: f64,
        /// Canvas point (0…1) the scale holds still.
        focus: Option<[f64; 2]>,
        /// Ease in and out.
        ease: bool,
    },
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
