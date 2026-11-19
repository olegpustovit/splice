// Splice code mode: the global `splice` inside code.run. Generated from the command registry
// and the MCP tool schemas. Tool calls are synchronous. Omit optional fields; never pass null
// to mean "unset" (null is only a value where a type says so).

/** A tool's business error, thrown by every `splice` call. `fix` says what to change. */
declare class SpliceError extends Error {
  code: string;
  fix: string;
  valid?: unknown[];
  current?: unknown;
}

declare namespace Splice {
  /** A frame number, a "$ref" bound earlier in the batch, or a position relative to an item, marker or word. */
  type At =
    | number
    | string
    | { item: string; edge?: "start" | "end"; offset?: number }
    | { ref: string; edge?: "start" | "end"; offset?: number }
    | { marker: string; offset?: number }
    | { word: string; transcriptHash?: string; edge?: "start" | "end"; offset?: number };
  /** A rectangle normalized to the source frame (0–1). */
  type Rect = { x: number; y: number; w: number; h: number };
  /** Curve toward the next key. */
  type Ease = "linear" | "hold" | "in" | "out" | "inOut" | [number, number, number, number];
  /** A keyframe; frames are relative to the owner's start. */
  type Key = { at: number; value: number; ease?: Ease };
  /** "r12" or 12. */
  type Revision = number | string;

  namespace op {
    interface Base {
      /** Binds the result for later ops as "$name" ("$name.new": its first created item). Pass "$music" (a bare name binds the same). */
      ref?: `$${string}`;
    }
    /** Split at Playhead (edit, 0.1). */
    interface ItemSplit extends Base {
      cmd: "item.split";
      /** clip or overlay */
      item: string;
      at: At;
    }
    /** Trim (edit, 0.1). */
    interface ItemTrim extends Base {
      cmd: "item.trim";
      item: string;
      edge: "in" | "out";
      /** new edge position */
      to?: At;
      /** frames to move the edge */
      delta?: number;
      /** @default true */
      ripple?: boolean;
    }
    /** Move (edit, 0.1). */
    interface ItemMove extends Base {
      cmd: "item.move";
      items: string[];
      /** new start (first item) */
      to?: At;
      /** frames */
      delta?: number;
      /** false = grave-key override · @default true */
      carryOverlays?: boolean;
      /** spine reorders magnetically · @default true */
      ripple?: boolean;
    }
    /** Delete (destructive, 0.1). */
    interface ItemDelete extends Base {
      cmd: "item.delete";
      items: string[];
      /** @default true */
      ripple?: boolean;
      /** keep leaves a gap clip carrying them · @default delete */
      overlays?: "delete" | "keep";
    }
    /** Duplicate (edit, 0.1). */
    interface ItemDuplicate extends Base {
      cmd: "item.duplicate";
      items: string[];
      /** paste position (default: right after the original) */
      at?: At;
    }
    /** Set Properties (edit, 0.1). */
    interface ItemSet extends Base {
      cmd: "item.set";
      items: string[];
      /** string|null */
      name?: string | null;
      color?: "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "gray" | null;
      fadeIn?: number;
      fadeOut?: number;
    }
    /** Speed (edit, 0.1). */
    interface ClipSpeed extends Base {
      cmd: "clip.speed";
      items: string[];
      /** number 0.25–16 */
      speed: number;
      /** @default true */
      ripple?: boolean;
    }
    /** Insert Media (edit, 0.1). */
    interface ClipInsert extends Base {
      cmd: "clip.insert";
      /** med_… (pass media or path) */
      media?: string;
      /** file path to import (pass path or media) */
      path?: string;
      /** frame (default: end of spine) */
      at?: At;
      /** @default true */
      ripple?: boolean;
      /** @default true (set false with path to register media only) */
      insert?: boolean;
      /** lan_… audio lane · audio files (music, voice-over, sound effects) become sound items on an audio lane (default: the first, created if missing) at frame 0 unless at is given; then audio.set for volume, duck and normalize */
      lane?: string;
    }
    /** Lift to Overlay (edit, 0.2). */
    interface ClipLift extends Base {
      cmd: "clip.lift";
      /** clip id[] */
      items: string[];
      /** lan_… (default: lowest free overlay lane) */
      lane?: string;
    }
    /** Replace Spine Clip (edit, 0.2). */
    interface OverlayReplaceSpine extends Base {
      cmd: "overlay.replaceSpine";
      /** ovl_… media overlay */
      item: string;
      /** clp_… (default: its anchor clip) */
      target?: string;
    }
    /** Add as Overlay (edit, 0.1). */
    interface OverlayAdd extends Base {
      cmd: "overlay.add";
      kind: "title" | "motion" | "media";
      /** med_… when kind=media */
      media?: string;
      /** frame or anchor form */
      at?: At;
      anchor?: { clip: string; sourceFrame?: number } | { pinned: true; frame?: number };
      /** lan_… (explicit lane may raise LANE_CONFLICT) */
      lane?: string;
      length?: number;
      /** title text */
      text?: string;
      /** scale */
      from?: number;
      /** scale */
      to?: number;
      /** {x,y} canvas point (0–1) a motion holds still */
      focus?: { x: number; y: number };
      /** ease a motion in and out */
      ease?: boolean;
      placement?: "full" | "card" | "corner-tl" | "corner-tr" | "corner-bl" | "corner-br";
      behindPresenter?: boolean;
      /** @default true */
      muted?: boolean;
      /** first media frame */
      source?: number;
      /** normalized source crop rectangle */
      crop?: Rect;
      fit?: "fit" | "fill";
    }
    /** Add Title (edit, 0.1). */
    interface TitleAdd extends Base {
      cmd: "title.add";
      /** frame or anchor form */
      at: At;
      /** @default 90 */
      length?: number;
      text?: string;
      /** lan_… */
      lane?: string;
    }
    /** Edit Title (edit, 0.1). */
    interface TitleSet extends Base {
      cmd: "title.set";
      /** ovl_… title */
      item: string;
      text: string;
    }
    /** Add Zoom (edit, 0.2). */
    interface ZoomAdd extends Base {
      cmd: "zoom.add";
      /** frame or anchor form */
      at: At;
      /** the camera holds the framing this long, then eases back to 1× · @default 90 */
      length?: number;
      /** 1–5 · magnification · @default 1.5 */
      amount?: number;
      /** {x,y} content point (0–1) to frame; the viewport never leaves the content */
      focus?: { x: number; y: number };
      /** content zooms inside a fixed window frame (portrait default), frame zooms the whole card */
      mode?: "auto" | "content" | "frame";
      /** @default focused */
      spring?: "focused" | "smooth" | "snappy" | "instant";
      /** lan_… camera lane */
      lane?: string;
    }
    /** Edit Zoom (edit, 0.2). */
    interface ZoomSet extends Base {
      cmd: "zoom.set";
      /** ovl_… zoom */
      zoom: string;
      /** 1–5 */
      amount?: number;
      /** {x,y} content point (0–1), or null to keep the current framing */
      focus?: { x: number; y: number } | null;
      mode?: "auto" | "content" | "frame";
      spring?: "focused" | "smooth" | "snappy" | "instant";
    }
    /** Pin / Unpin (edit, 0.1). */
    interface ItemAnchor extends Base {
      cmd: "item.anchor";
      /** ovl_… */
      item: string;
      /** pin at the current frame, or re-anchor to the clip under it */
      pinned?: boolean;
      anchor?: { clip: string; sourceFrame?: number } | { pinned: true; frame?: number };
    }
    /** Move to Lane (edit, 0.1). */
    interface ItemLane extends Base {
      cmd: "item.lane";
      /** ovl_…[] */
      items: string[];
      /** lan_… explicit (LANE_CONFLICT if occupied) */
      lane?: string;
      dir?: "up" | "down";
    }
    /** Overlay Settings (edit, 0.1). */
    interface OverlaySet extends Base {
      cmd: "overlay.set";
      /** ovl_… media */
      item: string;
      placement?: "full" | "card" | "corner-tl" | "corner-tr" | "corner-bl" | "corner-br";
      behindPresenter?: boolean;
      muted?: boolean;
    }
    /** New Lane (edit, 0.1). */
    interface LaneCreate extends Base {
      cmd: "lane.create";
      kind: "graphics" | "overlay" | "motion" | "captions" | "audio";
      name?: string;
    }
    /** Rename Lane (edit, 0.1). */
    interface LaneRename extends Base {
      cmd: "lane.rename";
      /** lan_… */
      lane: string;
      name: string;
    }
    /** Delete Lane (destructive, 0.1). */
    interface LaneDelete extends Base {
      cmd: "lane.delete";
      /** lan_… */
      lane: string;
    }
    /** Show / Hide Component (edit, 0.2). */
    interface ComponentSet extends Base {
      cmd: "component.set";
      /** clp_… */
      clip: string;
      component: "screen" | "camera" | "mic" | "system" | "pointer" | "keys";
      enabled: boolean;
    }
    /** Volume (edit, 0.2). */
    interface AudioSet extends Base {
      cmd: "audio.set";
      /** clp_… take, with component */
      clip?: string;
      component?: "mic" | "system";
      /** 0–2 · linear gain */
      volume?: number;
      /** ovl_…[] sound items on an audio lane (instead of clip) */
      items?: string[];
      /** {amount,attack,release} · amount 0–1, attack and release in frames; lowers sound items under speech, or null */
      duck?: { amount: number; attack?: number; release?: number } | null;
      /** number · LUFS target for sound items (e.g. -14 for social), from the measured loudness */
      normalize?: number;
    }
    /** Cursor Settings (edit, 0.2). */
    interface CursorSet extends Base {
      cmd: "cursor.set";
      /** clp_… */
      clip: string;
      /** 0.5–4 */
      size?: number;
      clickRings?: boolean;
    }
    /** Keystroke Chip Settings (edit, 0.2). */
    interface KeysSet extends Base {
      cmd: "keys.set";
      /** clp_… */
      clip: string;
      /** 0.5–3 */
      size?: number;
    }
    /** Hide Keystrokes (edit, 0.2). */
    interface KeysHide extends Base {
      cmd: "keys.hide";
      /** clp_… */
      clip: string;
      /** int[] · event indexes from takes.events */
      events: number[];
      hidden: boolean;
    }
    /** Add Marker (edit, 0.1). */
    interface MarkerAdd extends Base {
      cmd: "marker.add";
      /** frame or anchor form */
      at: At;
      name?: string;
    }
    /** Edit Marker (edit, 0.1). */
    interface MarkerSet extends Base {
      cmd: "marker.set";
      /** mrk_… */
      marker: string;
      name?: string;
      at?: At;
    }
    /** Output Shape (edit, 0.1). */
    interface ProjectSet extends Base {
      cmd: "project.set";
      aspect?: "9:16" | "4:5" | "1:1" | "4:3" | "16:9";
      access?: "off" | "read" | "live";
    }
    /** Crop (edit, 0.2). */
    interface ClipCrop extends Base {
      cmd: "clip.crop";
      /** spine clips or media overlays */
      items: string[];
      /** {x,y,w,h} normalized to the source frame (0–1), or null for the full frame */
      rect?: Rect | null;
      /** how the cropped source sits in the canvas (or overlay box) */
      fit?: "fit" | "fill";
      /** instead of rect: the largest crop of this shape, fit fill (reframe without bands) */
      aspect?: "9:16" | "4:5" | "1:1" | "4:3" | "16:9" | "canvas";
      /** {x,y} source point (0–1) the aspect crop keeps in frame (default: center) */
      focus?: { x: number; y: number };
    }
    /** Delete Range (destructive, 0.2). */
    interface CutRange extends Base {
      cmd: "cut.range";
      from: At;
      to: At;
      /** @default true */
      ripple?: boolean;
    }
    /** Delete Words (destructive, 0.2). */
    interface CutWords extends Base {
      cmd: "cut.words";
      /** word id[] */
      words: string[];
      /** string from transcript.get */
      transcriptHash?: string;
    }
    /** Add Effect (edit, 0.2). */
    interface EffectAdd extends Base {
      cmd: "effect.add";
      /** clp_… | ovl_… | lan_… | lan_spine | project */
      target: string;
      /** effect type · any key of Splice.Effects (effects.json): window, cursor, camera, captions, transform, color, mask, … */
      type: EffectType;
      /** only the values you want to set; the rest inherit */
      params?: Record<string, unknown>;
      /** false turns an inherited effect off at this scope · @default true */
      enabled?: boolean;
    }
    /** Set Effect Parameters (edit, 0.2). */
    interface EffectSet extends Base {
      cmd: "effect.set";
      /** fx_… id, or use target + type */
      effect?: string;
      /** owner id (creates the instance when missing) */
      target?: string;
      /** effect type with target */
      type?: EffectType;
      params?: Record<string, unknown>;
      enabled?: boolean;
      /** string[] · parameter names to drop so they inherit again */
      reset?: string[];
    }
    /** Remove Effect (edit, 0.2). */
    interface EffectRemove extends Base {
      cmd: "effect.remove";
      /** fx_…[] ids */
      effects?: string[];
      /** owner id · with type, or alone to remove all its effects */
      target?: string;
      /** effect type */
      type?: EffectType;
    }
    /** Paste Effects (edit, 0.2). */
    interface EffectPaste extends Base {
      cmd: "effect.paste";
      /** owner id to copy local effects from */
      from: string;
      /** owner id[] */
      to: string[];
      /** merge keeps the target's other values · @default merge */
      mode?: "merge" | "replace";
    }
    /** Promote Effect (edit, 0.2). */
    interface EffectPromote extends Base {
      cmd: "effect.promote";
      /** fx_… on a clip or lane */
      effect: string;
      to: "lane" | "project";
      /** also drop the same type's overrides on sibling clips · @default false */
      clearOverrides?: boolean;
    }
    /** Generate Captions (edit, 0.2). */
    interface CaptionsGenerate extends Base {
      cmd: "captions.generate";
      /** clp_…[] · takes to caption, including pieces split from them earlier in the batch (default: every clip with a transcript) */
      clips?: string[];
      /** int 1–8 · words per caption · @default 3 */
      maxWords?: number;
      /** caption um / uh too · @default false */
      fillers?: boolean;
      /** lan_… captions lane (default: the first, created if missing) */
      lane?: string;
      /** bold|karaoke|pop|box|clean|typewriter · sets the lane's Caption Style */
      style?: "bold" | "karaoke" | "pop" | "box" | "clean" | "typewriter";
    }
    /** Edit Caption (edit, 0.2). */
    interface CaptionSet extends Base {
      cmd: "caption.set";
      /** ovl_… caption */
      item: string;
      /** retype the cue; words map onto its words in order, extra text joins the last word, missing words are hidden */
      text?: string;
      /** {wordId:{text?, emph?, emoji?, hidden?} | null} · per-word display edits; null resets the word */
      words?: Record<string, { text?: string; emph?: boolean; emoji?: string; hidden?: boolean } | null>;
    }
    /** Split Caption (edit, 0.2). */
    interface CaptionSplit extends Base {
      cmd: "caption.split";
      /** ovl_… caption */
      item: string;
      /** word id that starts the new caption */
      word: string;
    }
    /** Merge Captions (edit, 0.2). */
    interface CaptionMerge extends Base {
      cmd: "caption.merge";
      /** ovl_…[] · two or more captions on one lane */
      items: string[];
    }
    /** Join Clips (edit, 0.3). */
    interface ClipJoin extends Base {
      cmd: "clip.join";
      /** clp_…[] · adjacent spine clips cut from one source, in order */
      items: string[];
    }
    /** Ungroup (edit, 0.3). */
    interface ClipUngroup extends Base {
      cmd: "clip.ungroup";
      /** clp_… · a clip made by clip.join */
      item: string;
    }
    /** Restore Cuts (edit, 0.3). */
    interface ClipRestore extends Base {
      cmd: "clip.restore";
      /** clp_…[] · clips whose cut material comes back; omit for every seam */
      items?: string[];
    }
    /** Reorder Lane (edit, 0.3). */
    interface LaneReorder extends Base {
      cmd: "lane.reorder";
      /** lan_… */
      lane: string;
      /** number · stack index, 0 at the top */
      to: number;
    }
    /** Animated Crop (edit, 0.3). */
    interface CropKeys extends Base {
      cmd: "crop.keys";
      /** clp_… | ovl_… · clip or media overlay */
      item: string;
      /** [{at, x, y, w, h, ease}] · owner frames; crop normalized to the source; ease linear, hold, in, out, inOut or [x1,y1,x2,y2] */
      keys?: { at: number; x: number; y: number; w: number; h: number; ease?: Ease }[];
      /** splice mcp computes keys that keep it in frame */
      follow?: "cursor" | "face";
      /** crop shape when following = canvas */
      aspect?: "9:16" | "4:5" | "1:1" | "4:3" | "16:9";
    }
    /** Add Transition (edit, 0.3). */
    interface TransitionAdd extends Base {
      cmd: "transition.add";
      /** clp_… · the incoming spine clip */
      item: string;
      /** @default dissolve */
      kind?: "dissolve" | "dip" | "wipe" | "push" | "slide" | "zoom";
      /** centered on the cut · @default 15 */
      length?: number;
      /** @default left */
      direction?: "left" | "right" | "up" | "down";
      /** #rrggbb · dip color */
      color?: string;
    }
    /** Add Mask (edit, 0.3). */
    interface MaskAdd extends Base {
      cmd: "mask.add";
      /** clp_… | ovl_… */
      item: string;
      shape?: "rect" | "roundRect" | "ellipse";
      mode?: "blur" | "pixelate" | "cover" | "spotlight";
      /** number · 0…1 of the picture */
      x?: number;
      /** number */
      y?: number;
      /** number */
      w?: number;
      /** number */
      h?: number;
      /** number · 0…1 */
      amount?: number;
      /** number */
      feather?: number;
      /** boolean */
      invert?: boolean;
      /** #rrggbb · cover color */
      color?: string;
    }
    /** Edit Mask (edit, 0.3). */
    interface MaskSet extends Base {
      cmd: "mask.set";
      /** fx_… · mask effect */
      mask: string;
      /** boolean */
      enabled?: boolean;
      /** number | {keys} */
      x?: number | { keys: Key[] };
      /** number | {keys} */
      y?: number | { keys: Key[] };
      /** number | {keys} */
      w?: number | { keys: Key[] };
      /** number | {keys} */
      h?: number | { keys: Key[] };
      /** number */
      amount?: number;
      mode?: "blur" | "pixelate" | "cover" | "spotlight";
      shape?: "rect" | "roundRect" | "ellipse";
      /** number */
      feather?: number;
      /** boolean */
      invert?: boolean;
      /** #rrggbb · cover color */
      color?: string;
    }
    /** Translate Captions (edit, 0.3). */
    interface CaptionsTranslate extends Base {
      cmd: "captions.translate";
      /** lan_… · captions lane */
      lane: string;
      /** BCP 47 tag */
      language: string;
      /** string[] · one per cue in order; splice mcp translates on device when omitted and a language pack is installed */
      texts?: string[];
    }
    /** Import SRT / VTT (edit, 0.3). */
    interface CaptionsImport extends Base {
      cmd: "captions.import";
      /** SRT or WebVTT text · cues align to transcript words */
      text: string;
      /** lan_… · destination captions lane; a new one when omitted */
      lane?: string;
    }
    /** Link Audio (edit, 0.3). */
    interface AudioLink extends Base {
      cmd: "audio.link";
      /** ovl_… · a sound made by audio.unlink */
      item: string;
    }
    /** Unlink Audio (edit, 0.3). */
    interface AudioUnlink extends Base {
      cmd: "audio.unlink";
      /** clp_… · spine clip */
      item: string;
      /** lan_… · destination audio lane */
      lane?: string;
    }
    /** Move into Sync (edit, 0.3). */
    interface AudioSync extends Base {
      cmd: "audio.sync";
      /** ovl_… · sound or media overlay */
      item: string;
      /** clp_… · spine clip it belongs to */
      to: string;
      /** item source frame at the clip's first source frame; splice mcp measures it when omitted */
      offset: number;
    }
    /** Style (edit, 0.3). */
    interface StyleSet extends Base {
      cmd: "style.set";
      font?: string;
      /** #rrggbb · primary */
      color?: string;
      /** #rrggbb · secondary / background */
      color2?: string;
      /** #rrggbb */
      accent?: string;
      /** #rrggbb */
      textColor?: string;
      /** path */
      logo?: string;
      logoPosition?: "none" | "topLeft" | "topRight" | "bottomLeft" | "bottomRight";
    }
    /** Cut Pauses (edit, 0.3). */
    interface CutPauses extends Base {
      cmd: "cut.pauses";
      /** pauses at least this long are cut · @default 0.7 s */
      min?: number;
      /** silence kept on each side · @default 4 */
      keep?: number;
    }
    /** Tighten Pace (edit, 0.3). */
    interface IntentPace extends Base {
      cmd: "intent.pace";
      /** longest pause left · @default 0.4 s */
      maxPause?: number;
    }
    /** Fit to Duration (edit, 0.3). */
    interface IntentFitDuration extends Base {
      cmd: "intent.fitDuration";
      /** number · target length; pauses go first, then fillers, then short sentences */
      seconds: number;
    }
    /** Keep Shots (edit, 0.3). */
    interface IntentKeepShots extends Base {
      cmd: "intent.keepShots";
      /** [[from, to]] · timeline frames to keep; the rest of the spine is cut */
      ranges: [number, number][];
    }
    /** Auto Zoom (edit, 0.3). */
    interface IntentAutoZoom extends Base {
      cmd: "intent.autoZoom";
      /** clp_… · screen take */
      item: string;
      /** [{at, x, y}] · source frames; splice mcp reads them from the take when omitted */
      clicks?: { at: number; x: number; y: number }[];
      /** [{from, to}] · source frames; typing extends a zoom and never moves it (read from the take when omitted) */
      typing?: { from: number; to: number }[];
      /** 1–5 · @default 1.5 */
      amount?: number;
    }
    /** Set Transcript (edit, 0.2). */
    interface TranscriptSet extends Base {
      cmd: "transcript.set";
      /** med_… the recorded or imported source */
      media: string;
      /** [{text, from, to}] source frames, in order · from transcription (transcript.get {transcribe:true} does this) */
      words: { text: string; from: number; to: number }[];
      /** BCP 47 tag · @default en-US */
      language?: string;
    }
    /** Roll Edit (edit, 0.3). */
    interface ItemRoll extends Base {
      cmd: "item.roll";
      /** clp_… · its out-edge and the next clip's in-edge move together */
      item: string;
      /** signed */
      by: number;
    }
    /** Slip (edit, 0.3). */
    interface ItemSlip extends Base {
      cmd: "item.slip";
      /** clp_… | ovl_… · clip or media overlay */
      item: string;
      /** signed, on the source clock */
      by: number;
    }
    /** Slide (edit, 0.3). */
    interface ItemSlide extends Base {
      cmd: "item.slide";
      /** clp_… · moves between its neighbors */
      item: string;
      /** signed */
      by: number;
    }
    /** Freeze Frame (edit, 0.3). */
    interface ClipFreeze extends Base {
      cmd: "clip.freeze";
      /** clp_… */
      item: string;
      /** timeline frame to hold */
      at: At;
      /** @default 60 */
      length?: number;
    }
    /** Compound Clip (edit, 0.3). */
    interface ClipCompound extends Base {
      cmd: "clip.compound";
      /** clp_…[] · adjacent spine clips; overlays anchored inside move with them */
      items: string[];
    }
    /** Break Apart (edit, 0.3). */
    interface ClipUncompound extends Base {
      cmd: "clip.uncompound";
      /** clp_… · compound clip */
      item: string;
    }
    /** Edit Transition (edit, 0.3). */
    interface TransitionSet extends Base {
      cmd: "transition.set";
      /** clp_… · the incoming spine clip */
      item: string;
      kind?: "dissolve" | "dip" | "wipe" | "push" | "slide" | "zoom";
      length?: number;
      direction?: "left" | "right" | "up" | "down";
      /** #rrggbb */
      color?: string;
    }
    /** Remove Transition (edit, 0.3). */
    interface TransitionRemove extends Base {
      cmd: "transition.remove";
      /** clp_… · the incoming spine clip */
      item: string;
    }
    /** Set Parameter (edit, 0.3). */
    interface ParamSet extends Base {
      cmd: "param.set";
      /** clp_… | ovl_… | lan_… | project */
      target: string;
      /** type.param · e.g. transform.scale, color.exposure, volume.gain */
      path: string;
      /** number */
      value: number;
    }
    /** Keyframe Parameter (edit, 0.3). */
    interface ParamKeys extends Base {
      cmd: "param.keys";
      /** clp_… | ovl_… | lan_… | project */
      target: string;
      /** type.param · any number in commands.describe effectTypes */
      path: string;
      /** [{at, value, ease}] · frames relative to the owner's start (absolute for lanes and the project) */
      keys: Key[];
      /** @default replace */
      mode?: "replace" | "merge";
    }
    /** Track Mask (edit, 0.3). */
    interface MaskTrack extends Base {
      cmd: "mask.track";
      /** fx_… · mask effect */
      mask: string;
      /** [{at, x, y}] · mask centers; splice mcp tracks the region when omitted */
      keys?: { at: number; x: number; y: number }[];
    }
    /** Add Shape (edit, 0.3). */
    interface GeneratorAdd extends Base {
      cmd: "generator.add";
      shape: "solid" | "gradient" | "rect" | "ellipse" | "arrow" | "line" | "callout";
      at: At;
      /** @default 90 */
      length?: number;
      /** {x, y, w, h} · normalized canvas; arrows run corner to corner */
      rect?: Rect;
      /** lan_… · graphics lane */
      lane?: string;
      /** #rrggbb */
      fill?: string;
      /** #rrggbb */
      fill2?: string;
      /** #rrggbb */
      stroke?: string;
      /** number */
      strokeWidth?: number;
      /** number */
      corner?: number;
    }
    /** Add Template (edit, 0.3). */
    interface TemplateAdd extends Base {
      cmd: "template.add";
      name: "lower-third" | "callout" | "cta" | "title-card" | "chapter-card";
      at: At;
      /** @default 4 s */
      length?: number;
      /** {title, subtitle} · text */
      fields?: { title?: string; subtitle?: string };
    }
    /** Edit Template (edit, 0.3). */
    interface TemplateSet extends Base {
      cmd: "template.set";
      /** ovl_… · any item of the template */
      item: string;
      /** {title, subtitle} · text */
      fields: { title?: string; subtitle?: string };
    }
    /** Relink Media (edit, 0.3). */
    interface MediaRelink extends Base {
      cmd: "media.relink";
      /** med_… */
      media: string;
      /** new file path */
      path: string;
      /** number · expected length, checked against the probed file */
      frames?: number;
      /** boolean · accept a different length or kind */
      force?: boolean;
    }
    /** Consolidate Media (edit, 0.3). */
    interface MediaConsolidate extends Base {
      cmd: "media.consolidate";
      /** med_…[] · omit for every external file */
      media?: string[];
    }
    /** Remove Zoom (edit, 0.2). */
    interface ZoomRemove extends Base {
      cmd: "zoom.remove";
      /** ovl_… zoom · the camera springs back to 1× where it was */
      zoom: string;
    }
  }
  /** Effect types and parameters (effects.json, the validator's own schema). */
  interface Effects {
    /** Window: spine clips and media overlays. */
    window: {
      /** Padding · 0–0.3 · @default 0.07 */
      padding?: number;
      /** Corners · 0–0.12 · @default 0.03 */
      corner?: number;
      /** Shadow · 0–1 · @default 0.6 */
      shadow?: number;
      /** Inset · 0–0.02 · @default 0 */
      inset?: number;
      /** Background · @default "gradient" */
      background?: "gradient" | "color" | "none";
      /** Color · #rrggbb · @default "#5b6f63" */
      color?: string;
      /** Color 2 · #rrggbb · @default "#1d2420" */
      color2?: string;
    };
    /** Cursor: screen takes. */
    cursor: {
      /** Size · 0.5–4 · @default 1.5 */
      size?: number;
      /** Click rings · @default true */
      clickRings?: boolean;
      /** Hide cursor · @default false */
      hidden?: boolean;
    };
    /** Camera Layout: screen takes. */
    camera: {
      /** Corner · @default "br" */
      corner?: "br" | "bl" | "tr" | "tl";
      /** Size · 0.12–0.45 · @default 0.24 */
      size?: number;
      /** Roundness · 0–1 · @default 0.2 */
      roundness?: number;
      /** Shadow · 0–1 · @default 0.5 */
      shadow?: number;
      /** Background · @default "keep" */
      background?: "keep" | "blur" | "remove";
    };
    /** Caption Style: captions lanes, caption cues and the project. */
    captions: {
      /** Preset · @default "bold" */
      preset?: "bold" | "karaoke" | "pop" | "box" | "clean" | "typewriter" | "neon" | "comic" | "minimal" | "subtitle" | "highlight" | "outline" | "shadow" | "bubble" | "gaming" | "news" | "podcast" | "cinematic" | "glow" | "retro" | "elegant" | "hype";
      /** Size · 0.03–0.14 · @default 0.075 */
      size?: number;
      /** Position · 0.1–0.9 · @default 0.68 */
      position?: number;
      /** Text · #rrggbb · @default "#ffffff" */
      color?: string;
      /** Highlight · #rrggbb · @default "#ffd23f" */
      active?: string;
      /** Stroke · 0–0.3 · @default 0.14 */
      stroke?: number;
      /** Uppercase · @default true */
      uppercase?: boolean;
      /** Safe zone · @default "tiktok" */
      safe?: "tiktok" | "reels" | "shorts" | "none";
    };
    /** Keystroke Chips: screen takes. */
    keystrokes: {
      /** Size · 0.5–3 · @default 1 */
      size?: number;
      /** Position · @default "bottom-left" */
      position?: "bottom-left" | "bottom-center" | "top-left";
    };
    /** Transform: spine clips, media overlays, titles, shapes and captions. */
    transform: {
      /** X · -2–2 · @default 0 */
      x?: number | { keys: Key[] };
      /** Y · -2–2 · @default 0 */
      y?: number | { keys: Key[] };
      /** Scale · 0.05–10 · @default 1 */
      scale?: number | { keys: Key[] };
      /** Rotation · -720–720 · @default 0 */
      rotation?: number | { keys: Key[] };
      /** Opacity · 0–1 · @default 1 */
      opacity?: number | { keys: Key[] };
      /** Anchor X · 0–1 · @default 0.5 */
      anchorX?: number | { keys: Key[] };
      /** Anchor Y · 0–1 · @default 0.5 */
      anchorY?: number | { keys: Key[] };
      /** Blend · @default "normal" */
      blend?: "normal" | "multiply" | "screen" | "overlay" | "add" | "darken" | "lighten";
    };
    /** Animated Crop: spine clips and media overlays. */
    crop: {
      /** Left · 0–1 · @default 0 */
      x?: number | { keys: Key[] };
      /** Top · 0–1 · @default 0 */
      y?: number | { keys: Key[] };
      /** Width · 0.01–1 · @default 1 */
      w?: number | { keys: Key[] };
      /** Height · 0.01–1 · @default 1 */
      h?: number | { keys: Key[] };
    };
    /** Color: spine clips, media overlays, lanes and the project. */
    color: {
      /** Exposure · -3–3 · @default 0 */
      exposure?: number | { keys: Key[] };
      /** Contrast · 0–2 · @default 1 */
      contrast?: number | { keys: Key[] };
      /** Saturation · 0–2 · @default 1 */
      saturation?: number | { keys: Key[] };
      /** Vibrance · -1–1 · @default 0 */
      vibrance?: number | { keys: Key[] };
      /** Temperature · -1–1 · @default 0 */
      temperature?: number | { keys: Key[] };
      /** Tint · -1–1 · @default 0 */
      tint?: number | { keys: Key[] };
      /** Highlights · -1–1 · @default 0 */
      highlights?: number | { keys: Key[] };
      /** Shadows · -1–1 · @default 0 */
      shadows?: number | { keys: Key[] };
      /** LUT (.cube) · @default "" */
      lut?: string;
      /** LUT amount · 0–1 · @default 1 */
      lutAmount?: number | { keys: Key[] };
    };
    /** Mask: spine clips and media overlays. */
    mask: {
      /** Shape · @default "rect" */
      shape?: "rect" | "roundRect" | "ellipse";
      /** Left · -1–2 · @default 0.25 */
      x?: number | { keys: Key[] };
      /** Top · -1–2 · @default 0.25 */
      y?: number | { keys: Key[] };
      /** Width · 0–2 · @default 0.5 */
      w?: number | { keys: Key[] };
      /** Height · 0–2 · @default 0.5 */
      h?: number | { keys: Key[] };
      /** Mode · @default "blur" */
      mode?: "blur" | "pixelate" | "cover" | "spotlight";
      /** Amount · 0–1 · @default 0.6 */
      amount?: number | { keys: Key[] };
      /** Feather · 0–0.2 · @default 0.01 */
      feather?: number | { keys: Key[] };
      /** Invert · @default false */
      invert?: boolean;
      /** Cover color · #rrggbb · @default "#000000" */
      color?: string;
    };
    /** Chroma Key: spine clips and media overlays. */
    key: {
      /** Key color · #rrggbb · @default "#00ff00" */
      color?: string;
      /** Tolerance · 0–1 · @default 0.3 */
      tolerance?: number | { keys: Key[] };
      /** Softness · 0–1 · @default 0.1 */
      softness?: number | { keys: Key[] };
      /** Spill · 0–1 · @default 0.5 */
      spill?: number | { keys: Key[] };
    };
    /** Background Removal: spine clips and media overlays with people. */
    cutout: {
      /** Background · @default "remove" */
      background?: "remove" | "blur";
      /** Amount · 0–1 · @default 1 */
      amount?: number | { keys: Key[] };
    };
    /** Transition: spine clips (into the clip from the one before). */
    transition: {
      /** Kind · @default "dissolve" */
      kind?: "dissolve" | "dip" | "wipe" | "push" | "slide" | "zoom";
      /** Length · 2–240 · @default 15 */
      length?: number;
      /** Direction · @default "left" */
      direction?: "left" | "right" | "up" | "down";
      /** Dip color · #rrggbb · @default "#000000" */
      color?: string;
    };
    /** Speed Ramp: spine clips. */
    speed: {
      /** Rate (0 holds the frame) · 0–16 · @default 1 */
      rate?: number | { keys: Key[] };
      /** Keep pitch · @default true */
      pitch?: boolean;
    };
    /** Volume: clips with audio, sounds, audio lanes, the spine and the project. */
    volume: {
      /** Gain · 0–4 · @default 1 */
      gain?: number | { keys: Key[] };
      /** Pan · -1–1 · @default 0 */
      pan?: number | { keys: Key[] };
      /** Mute · @default false */
      mute?: boolean;
      /** Solo · @default false */
      solo?: boolean;
    };
    /** EQ: clips with audio, sounds and audio lanes. */
    eq: {
      /** Low cut · 20–400 · @default 20 */
      lowCut?: number;
      /** Low · -12–12 · @default 0 */
      low?: number;
      /** Mid · -12–12 · @default 0 */
      mid?: number;
      /** Mid frequency · 200–5000 · @default 1000 */
      midFrequency?: number;
      /** High · -12–12 · @default 0 */
      high?: number;
    };
    /** Compressor: clips with audio, sounds and audio lanes. */
    compressor: {
      /** Threshold · -60–0 · @default -20 */
      threshold?: number;
      /** Ratio · 1–20 · @default 3 */
      ratio?: number;
      /** Makeup gain · 0–24 · @default 0 */
      gain?: number;
    };
    /** Noise Gate: clips with audio, sounds and audio lanes. */
    gate: {
      /** Threshold · -80–0 · @default -50 */
      threshold?: number;
    };
    /** Voice Isolation: clips with audio and sounds. */
    voiceIsolation: {
      /** Amount · 0–1 · @default 1 */
      amount?: number;
    };
    /** Text Style: titles, captions lanes, caption cues and the project. */
    text: {
      /** Font · @default "SF Pro Display" */
      font?: string;
      /** Weight · @default "bold" */
      weight?: "regular" | "medium" | "semibold" | "bold" | "heavy" | "black";
      /** Size · 0.02–0.3 · @default 0.06 */
      size?: number;
      /** Color · #rrggbb · @default "#ffffff" */
      color?: string;
      /** Stroke · #rrggbb · @default "#000000" */
      stroke?: string;
      /** Stroke width · 0–0.3 · @default 0 */
      strokeWidth?: number;
      /** Shadow · 0–1 · @default 0.3 */
      shadow?: number;
      /** Box · @default "none" */
      box?: "none" | "solid" | "rounded";
      /** Box color · #rrggbb · @default "#000000" */
      boxColor?: string;
      /** Align · @default "center" */
      align?: "left" | "center" | "right";
      /** Line height · 0.8–2 · @default 1.15 */
      lineHeight?: number;
      /** Position · @default "top" */
      position?: "top" | "middle" | "bottom" | "lowerThird";
    };
    /** Text Animation: titles, captions lanes, caption cues and the project. */
    animate: {
      /** Preset · @default "fade" */
      preset?: "none" | "fade" | "rise" | "pop" | "typewriter" | "highlight" | "bounce" | "slide";
      /** Unit · @default "word" */
      unit?: "word" | "character" | "line";
      /** Duration · 1–60 · @default 8 */
      duration?: number;
      /** Stagger · 0–30 · @default 3 */
      stagger?: number;
    };
    /** Shape Style: shapes. */
    shape: {
      /** Fill · #rrggbb · @default "#ffffff" */
      fill?: string;
      /** Fill 2 · #rrggbb · @default "#ffffff" */
      fill2?: string;
      /** Stroke · #rrggbb · @default "#000000" */
      stroke?: string;
      /** Stroke width · 0–0.05 · @default 0 */
      strokeWidth?: number;
      /** Corner · 0–0.5 · @default 0 */
      corner?: number;
    };
    /** Template: items made by template.add. */
    template: {
      /** Template · @default "lower-third" */
      name?: "lower-third" | "callout" | "cta" | "title-card" | "chapter-card";
      /** Group · @default "" */
      group?: string;
    };
    /** Brand Kit: the project. */
    brand: {
      /** Font · @default "SF Pro Display" */
      font?: string;
      /** Primary · #rrggbb · @default "#ffffff" */
      color?: string;
      /** Secondary · #rrggbb · @default "#1d2420" */
      color2?: string;
      /** Accent · #rrggbb · @default "#ffd23f" */
      accent?: string;
      /** Text · #rrggbb · @default "#ffffff" */
      textColor?: string;
      /** Logo · @default "" */
      logo?: string;
      /** Logo position · @default "topRight" */
      logoPosition?: "none" | "topLeft" | "topRight" | "bottomLeft" | "bottomRight";
    };
  }
  type EffectType = keyof Effects;
  /** Any registry command, for edit.apply's ops. */
  type Op =
    | op.ItemSplit
    | op.ItemTrim
    | op.ItemMove
    | op.ItemDelete
    | op.ItemDuplicate
    | op.ItemSet
    | op.ClipSpeed
    | op.ClipInsert
    | op.ClipLift
    | op.OverlayReplaceSpine
    | op.OverlayAdd
    | op.TitleAdd
    | op.TitleSet
    | op.ZoomAdd
    | op.ZoomSet
    | op.ItemAnchor
    | op.ItemLane
    | op.OverlaySet
    | op.LaneCreate
    | op.LaneRename
    | op.LaneDelete
    | op.ComponentSet
    | op.AudioSet
    | op.CursorSet
    | op.KeysSet
    | op.KeysHide
    | op.MarkerAdd
    | op.MarkerSet
    | op.ProjectSet
    | op.ClipCrop
    | op.CutRange
    | op.CutWords
    | op.EffectAdd
    | op.EffectSet
    | op.EffectRemove
    | op.EffectPaste
    | op.EffectPromote
    | op.CaptionsGenerate
    | op.CaptionSet
    | op.CaptionSplit
    | op.CaptionMerge
    | op.ClipJoin
    | op.ClipUngroup
    | op.ClipRestore
    | op.LaneReorder
    | op.CropKeys
    | op.TransitionAdd
    | op.MaskAdd
    | op.MaskSet
    | op.CaptionsTranslate
    | op.CaptionsImport
    | op.AudioLink
    | op.AudioUnlink
    | op.AudioSync
    | op.StyleSet
    | op.CutPauses
    | op.IntentPace
    | op.IntentFitDuration
    | op.IntentKeepShots
    | op.IntentAutoZoom
    | op.TranscriptSet
    | op.ItemRoll
    | op.ItemSlip
    | op.ItemSlide
    | op.ClipFreeze
    | op.ClipCompound
    | op.ClipUncompound
    | op.TransitionSet
    | op.TransitionRemove
    | op.ParamSet
    | op.ParamKeys
    | op.MaskTrack
    | op.GeneratorAdd
    | op.TemplateAdd
    | op.TemplateSet
    | op.MediaRelink
    | op.MediaConsolidate
    | op.ZoomRemove;
}

declare const splice: {
  /** The projectId and sessionId passed to code.run, else the first project this code opens and session it
   * starts. Tool calls default to them. */
  readonly projectId: string | undefined;
  readonly sessionId: string | undefined;
  /** Calls a tool by name and returns its structured result. */
  call(name: string, args?: object): any;
  /** These declarations, filtered by a substring of a command or tool name. */
  describe(query?: string): string;
  /** Op builders: `splice.op.item.split({item, at})` returns `{cmd: "item.split", item, at}`. */
  op: {
    item: {
      split(args: Omit<Splice.op.ItemSplit, "cmd">): Splice.op.ItemSplit;
      trim(args: Omit<Splice.op.ItemTrim, "cmd">): Splice.op.ItemTrim;
      move(args: Omit<Splice.op.ItemMove, "cmd">): Splice.op.ItemMove;
      delete(args: Omit<Splice.op.ItemDelete, "cmd">): Splice.op.ItemDelete;
      duplicate(args: Omit<Splice.op.ItemDuplicate, "cmd">): Splice.op.ItemDuplicate;
      set(args: Omit<Splice.op.ItemSet, "cmd">): Splice.op.ItemSet;
      anchor(args: Omit<Splice.op.ItemAnchor, "cmd">): Splice.op.ItemAnchor;
      lane(args: Omit<Splice.op.ItemLane, "cmd">): Splice.op.ItemLane;
      roll(args: Omit<Splice.op.ItemRoll, "cmd">): Splice.op.ItemRoll;
      slip(args: Omit<Splice.op.ItemSlip, "cmd">): Splice.op.ItemSlip;
      slide(args: Omit<Splice.op.ItemSlide, "cmd">): Splice.op.ItemSlide;
    };
    clip: {
      speed(args: Omit<Splice.op.ClipSpeed, "cmd">): Splice.op.ClipSpeed;
      insert(args: Omit<Splice.op.ClipInsert, "cmd">): Splice.op.ClipInsert;
      lift(args: Omit<Splice.op.ClipLift, "cmd">): Splice.op.ClipLift;
      crop(args: Omit<Splice.op.ClipCrop, "cmd">): Splice.op.ClipCrop;
      join(args: Omit<Splice.op.ClipJoin, "cmd">): Splice.op.ClipJoin;
      ungroup(args: Omit<Splice.op.ClipUngroup, "cmd">): Splice.op.ClipUngroup;
      restore(args: Omit<Splice.op.ClipRestore, "cmd">): Splice.op.ClipRestore;
      freeze(args: Omit<Splice.op.ClipFreeze, "cmd">): Splice.op.ClipFreeze;
      compound(args: Omit<Splice.op.ClipCompound, "cmd">): Splice.op.ClipCompound;
      uncompound(args: Omit<Splice.op.ClipUncompound, "cmd">): Splice.op.ClipUncompound;
    };
    overlay: {
      replaceSpine(args: Omit<Splice.op.OverlayReplaceSpine, "cmd">): Splice.op.OverlayReplaceSpine;
      add(args: Omit<Splice.op.OverlayAdd, "cmd">): Splice.op.OverlayAdd;
      set(args: Omit<Splice.op.OverlaySet, "cmd">): Splice.op.OverlaySet;
    };
    title: {
      add(args: Omit<Splice.op.TitleAdd, "cmd">): Splice.op.TitleAdd;
      set(args: Omit<Splice.op.TitleSet, "cmd">): Splice.op.TitleSet;
    };
    zoom: {
      add(args: Omit<Splice.op.ZoomAdd, "cmd">): Splice.op.ZoomAdd;
      set(args: Omit<Splice.op.ZoomSet, "cmd">): Splice.op.ZoomSet;
      remove(args: Omit<Splice.op.ZoomRemove, "cmd">): Splice.op.ZoomRemove;
    };
    lane: {
      create(args: Omit<Splice.op.LaneCreate, "cmd">): Splice.op.LaneCreate;
      rename(args: Omit<Splice.op.LaneRename, "cmd">): Splice.op.LaneRename;
      delete(args: Omit<Splice.op.LaneDelete, "cmd">): Splice.op.LaneDelete;
      reorder(args: Omit<Splice.op.LaneReorder, "cmd">): Splice.op.LaneReorder;
    };
    component: {
      set(args: Omit<Splice.op.ComponentSet, "cmd">): Splice.op.ComponentSet;
    };
    audio: {
      set(args: Omit<Splice.op.AudioSet, "cmd">): Splice.op.AudioSet;
      link(args: Omit<Splice.op.AudioLink, "cmd">): Splice.op.AudioLink;
      unlink(args: Omit<Splice.op.AudioUnlink, "cmd">): Splice.op.AudioUnlink;
      sync(args: Omit<Splice.op.AudioSync, "cmd">): Splice.op.AudioSync;
    };
    cursor: {
      set(args: Omit<Splice.op.CursorSet, "cmd">): Splice.op.CursorSet;
    };
    keys: {
      set(args: Omit<Splice.op.KeysSet, "cmd">): Splice.op.KeysSet;
      hide(args: Omit<Splice.op.KeysHide, "cmd">): Splice.op.KeysHide;
    };
    marker: {
      add(args: Omit<Splice.op.MarkerAdd, "cmd">): Splice.op.MarkerAdd;
      set(args: Omit<Splice.op.MarkerSet, "cmd">): Splice.op.MarkerSet;
    };
    project: {
      set(args: Omit<Splice.op.ProjectSet, "cmd">): Splice.op.ProjectSet;
    };
    cut: {
      range(args: Omit<Splice.op.CutRange, "cmd">): Splice.op.CutRange;
      words(args: Omit<Splice.op.CutWords, "cmd">): Splice.op.CutWords;
      pauses(args: Omit<Splice.op.CutPauses, "cmd">): Splice.op.CutPauses;
    };
    effect: {
      add(args: Omit<Splice.op.EffectAdd, "cmd">): Splice.op.EffectAdd;
      set(args: Omit<Splice.op.EffectSet, "cmd">): Splice.op.EffectSet;
      remove(args: Omit<Splice.op.EffectRemove, "cmd">): Splice.op.EffectRemove;
      paste(args: Omit<Splice.op.EffectPaste, "cmd">): Splice.op.EffectPaste;
      promote(args: Omit<Splice.op.EffectPromote, "cmd">): Splice.op.EffectPromote;
    };
    captions: {
      generate(args: Omit<Splice.op.CaptionsGenerate, "cmd">): Splice.op.CaptionsGenerate;
      translate(args: Omit<Splice.op.CaptionsTranslate, "cmd">): Splice.op.CaptionsTranslate;
      import(args: Omit<Splice.op.CaptionsImport, "cmd">): Splice.op.CaptionsImport;
    };
    caption: {
      set(args: Omit<Splice.op.CaptionSet, "cmd">): Splice.op.CaptionSet;
      split(args: Omit<Splice.op.CaptionSplit, "cmd">): Splice.op.CaptionSplit;
      merge(args: Omit<Splice.op.CaptionMerge, "cmd">): Splice.op.CaptionMerge;
    };
    crop: {
      keys(args: Omit<Splice.op.CropKeys, "cmd">): Splice.op.CropKeys;
    };
    transition: {
      add(args: Omit<Splice.op.TransitionAdd, "cmd">): Splice.op.TransitionAdd;
      set(args: Omit<Splice.op.TransitionSet, "cmd">): Splice.op.TransitionSet;
      remove(args: Omit<Splice.op.TransitionRemove, "cmd">): Splice.op.TransitionRemove;
    };
    mask: {
      add(args: Omit<Splice.op.MaskAdd, "cmd">): Splice.op.MaskAdd;
      set(args: Omit<Splice.op.MaskSet, "cmd">): Splice.op.MaskSet;
      track(args: Omit<Splice.op.MaskTrack, "cmd">): Splice.op.MaskTrack;
    };
    style: {
      set(args: Omit<Splice.op.StyleSet, "cmd">): Splice.op.StyleSet;
    };
    intent: {
      pace(args: Omit<Splice.op.IntentPace, "cmd">): Splice.op.IntentPace;
      fitDuration(args: Omit<Splice.op.IntentFitDuration, "cmd">): Splice.op.IntentFitDuration;
      keepShots(args: Omit<Splice.op.IntentKeepShots, "cmd">): Splice.op.IntentKeepShots;
      autoZoom(args: Omit<Splice.op.IntentAutoZoom, "cmd">): Splice.op.IntentAutoZoom;
    };
    transcript: {
      set(args: Omit<Splice.op.TranscriptSet, "cmd">): Splice.op.TranscriptSet;
    };
    param: {
      set(args: Omit<Splice.op.ParamSet, "cmd">): Splice.op.ParamSet;
      keys(args: Omit<Splice.op.ParamKeys, "cmd">): Splice.op.ParamKeys;
    };
    generator: {
      add(args: Omit<Splice.op.GeneratorAdd, "cmd">): Splice.op.GeneratorAdd;
    };
    template: {
      add(args: Omit<Splice.op.TemplateAdd, "cmd">): Splice.op.TemplateAdd;
      set(args: Omit<Splice.op.TemplateSet, "cmd">): Splice.op.TemplateSet;
    };
    media: {
      relink(args: Omit<Splice.op.MediaRelink, "cmd">): Splice.op.MediaRelink;
      consolidate(args: Omit<Splice.op.MediaConsolidate, "cmd">): Splice.op.MediaConsolidate;
    };
  };
  project: {
    /** Open a .splice project by path or inspect an open project by ID. Use create:true to make a new project. */
    info(args?: { projectId?: string; path?: string; create?: boolean; saveAs?: string; }): { projectId: string; revision: Splice.Revision; fps: number; size?: unknown[]; aspect?: string; durationFrames: number; access: string; lanes: unknown[]; media: unknown[]; offlineMedia?: unknown[]; transcript?: Record<string, unknown>; effects?: Record<string, unknown>; openSession?: string | null; };
  };
  timeline: {
    /** List the spine, drawing lanes and markers with resolved frame ranges. detail:full adds every item field and is large; prefer concise, then items.inspect for the IDs you need. */
    snapshot(args?: { projectId?: string; from?: number; to?: number; detail?: "concise" | "full"; limit?: number; cursor?: string; }): { revision: Splice.Revision; fps?: number; range?: Record<string, unknown>; lanes: unknown[]; spine: unknown[]; markers: unknown[]; total?: number; nextCursor?: string | null; };
    /** Find items, markers, words and edit points by text, kind, lane, color or frame range. */
    find(args?: { projectId?: string; text?: string; kind?: string; lane?: string; color?: string; from?: number; to?: number; limit?: number; }): { revision: Splice.Revision; matches: unknown[]; total: number; };
  };
  items: {
    /** Inspect up to 50 objects by ID, including anchors, resolved frames, children and inherited effects. */
    inspect(args: { projectId?: string; ids: string[]; items?: string[]; }): { revision: Splice.Revision; items: unknown[]; };
  };
  takes: {
    /** Read the take's cursor, clicks and keys in timeline frames, with keystrokes redacted by project policy. */
    events(args: { projectId?: string; clip: string; from?: number; to?: number; }): { revision: Splice.Revision; clip: string; cursor?: unknown[]; clicks: unknown[]; keys: unknown[]; };
  };
  transcript: {
    /** Page through words by ID or get caption cues as JSON, SRT or WebVTT. Use the returned hash with word anchors. With transcribe:true (and sessionId), first recognizes the clip's speech on device and saves the words. */
    get(args?: { projectId?: string; clip?: string; item?: string; transcribe?: boolean; async?: boolean; language?: string; sessionId?: string; cursor?: string; limit?: number; captions?: "json" | "srt" | "vtt"; lane?: string; }): { revision: Splice.Revision; hash: string | null; clip?: string; words: unknown[]; fillers?: string[]; nextCursor?: string | null; lanes?: unknown[]; cues?: unknown[] | number; format?: "srt" | "vtt"; text?: string; };
  };
  media: {
    /** Analyze a clip's media, in timeline frames: loudness and pauses (default); peaks, beats and sound labels (speech, music, laughter); shot cuts, on-screen text with boxes, faces and people every second; matte builds the person matte for a cutout effect. Results are cached per file. Native kinds need macOS. */
    analyze(args: { projectId?: string; clip: string; kinds?: ("loudness" | "pauses" | "peaks" | "beats" | "sounds" | "shots" | "text" | "faces" | "people" | "matte")[]; async?: boolean; }): { revision: Splice.Revision; clip: string; components?: unknown[]; pauses?: unknown[]; peaks?: Record<string, unknown>; beats?: unknown[]; sounds?: unknown[]; shots?: unknown[]; text?: unknown[]; faces?: unknown[]; people?: unknown[]; matte?: string; };
  };
  view: {
    /** Render PNG frames from the export composition. Pass explicit frame numbers or a filmstrip count; sheet:true returns one contact-sheet image of them all. macOS builds only; elsewhere it fails with UNSUPPORTED_PLATFORM. */
    frames(args?: { projectId?: string; at?: number[]; count?: number; sheet?: boolean; }): { revision: Splice.Revision; frames: unknown[]; sheet?: string; };
  };
  commands: {
    /** Search the command registry. Pass an exact ID to see parameters, release tier and an example. */
    describe(args?: { projectId?: string; id?: string; query?: string; format?: "json" | "ts"; }): { commands: unknown[]; id?: string; title?: string; group?: string; risk?: string; cut?: string; available?: boolean; planned?: boolean; params?: Record<string, unknown>; shortcut?: string | null; ui?: unknown[]; example?: Record<string, unknown>; captionModel?: string; presets?: string[]; effectTypes?: Record<string, unknown>; resolution?: string; format?: "ts"; text?: string; };
  };
  session: {
    /** Start an agent editing session and get its checkpoint revision and session ID. */
    start(args?: { projectId?: string; agent?: string; }): { sessionId: string; checkpoint: Splice.Revision; revision: Splice.Revision; access?: string; resumed?: boolean; };
  };
  edit: {
    /** Apply one atomic batch of registry commands. Use dryRun to preview and idempotencyKey for a safe retry. */
    apply(args: { projectId?: string; sessionId?: string; ifRevision: Splice.Revision; ops: Splice.Op[]; summary?: string; dryRun?: boolean; idempotencyKey?: string; }): { revision: Splice.Revision; dryRun?: boolean; replayed?: boolean; changes: unknown[]; sideEffects: unknown[]; diff: unknown[]; };
  };
  history: {
    /** List revision labels, sources and affected IDs. Filter by session to inspect its batch history. */
    list(args?: { projectId?: string; sessionId?: string; limit?: number; cursor?: string; }): { sessionId?: string; revision: Splice.Revision; batches: unknown[]; open?: boolean; checkpoint?: Splice.Revision; humanEditsSince?: number; nextCursor?: string | null; };
    /** Create an undo revision for the session's last edit, the whole session, or an exact earlier revision. */
    undo(args: { projectId?: string; sessionId?: string; ifRevision: Splice.Revision; scope: "last" | "session" | "to"; revision?: Splice.Revision; dryRun?: boolean; }): { revision: Splice.Revision; skipped?: unknown[]; replayed?: number; reverted?: unknown[]; restored?: string | null; };
  };
  ui: {
    /** Set playhead, selection, range or zoom in an open viewer. Headless calls return applied:false. */
    show(args?: { projectId?: string; at?: number | Record<string, unknown>; select?: string[]; range?: Record<string, unknown>; expand?: string; view?: string; viewer?: Record<string, unknown>; }): { applied: boolean; reason?: string; revision?: Splice.Revision; playhead?: number; selection?: string[]; };
  };
  export: {
    /** Export the timeline: presets for social (youtube-1080/4k, shorts, reels, tiktok, linkedin), masters (prores-422, prores-4444 and hevc-alpha keep transparency), audio (audio-m4a, audio-wav), gif and poster (done at once). from/to or marker limit the range; markers become chapters; captions:"srt" writes a sidecar; variants:[{aspect, preset}] renders several aspects, each reframed to fill; format:"fcpxml" writes the cut for Final Cut Pro (every platform). Video and audio need macOS and run as jobs (two at a time). Without sessionId the export opens its own session. */
    start(args: { projectId?: string; sessionId?: string; ifRevision: Splice.Revision; preset?: "social" | "social-h264" | "social-hevc" | "youtube-1080" | "youtube-4k" | "shorts" | "reels" | "tiktok" | "linkedin" | "prores-422" | "prores-4444" | "hevc-alpha" | "audio-m4a" | "audio-wav" | "gif" | "poster"; format?: "video" | "fcpxml"; from?: number; to?: number; marker?: string; bitrate?: number; captions?: "srt" | "vtt"; chapters?: boolean; variants?: Record<string, unknown>[]; path?: string; overwrite?: boolean; }): { jobId: string; state: string; settings?: Record<string, unknown>; output?: string; jobs?: unknown[]; };
  };
  jobs: {
    /** Check the progress and result of an export, transcription or recording job. wait (seconds, up to 120) returns once the job stops running, so one call (or one code.run: waiting is not counted against its 2 s) sees it done; without wait, each call advances a running export by up to 50 ms. */
    status(args: { projectId?: string; jobId: string; wait?: number; }): { jobId: string; id?: string; kind?: string; state: string; progress?: number; settings?: Record<string, unknown>; output?: string; elapsed?: number; bytes?: number; droppedFrames?: number; fps?: number; eta?: number; revision?: Splice.Revision; };
    /** Cancel a running or pending job. */
    cancel(args: { projectId?: string; jobId: string; sessionId?: string; ifRevision?: Splice.Revision; }): { jobId: string; id?: string; state: string; };
  };
  record: {
    /** Start headless capture of a display, window or area. To record an app, pass source:{window:{app:"Safari"}} (app name or bundle ID; add title when it has several windows). dryRun lists sources, microphones, cameras and permissions; window titles only with titles:true. voiceover:true records the mic onto a new audio lane at frame at; camera:true with screen:false is a camera-only take; retakeOf places the take after an earlier one; exclude hides apps. resume:jobId resumes a take paused with record.stop {pause:true}; resume with marker drops a marker. recover:true publishes takes a crash left staged. */
    start(args?: { projectId?: string; path?: string; create?: boolean; sessionId?: string; ifRevision?: Splice.Revision; source?: Record<string, unknown>; display?: string | number; window?: string | number | Record<string, unknown>; area?: Record<string, unknown>; mic?: boolean; systemAudio?: boolean; camera?: boolean; events?: boolean; insert?: boolean; still?: boolean; countdown?: number; dryRun?: boolean; duration?: number; voiceover?: boolean; at?: number; screen?: boolean; retakeOf?: string; exclude?: string[]; resume?: string; marker?: string; recover?: boolean; titles?: boolean; }): { jobId: string; state: string; projectId?: string; sessionId?: string; settings?: Record<string, unknown>; sources?: Record<string, unknown>; devices?: unknown[]; recovered?: unknown[]; markers?: unknown[]; permissions?: Record<string, unknown>; identity?: Record<string, unknown>; };
    /** Finish a recording (started with record.start, e.g. source:{window:{app}}) and insert the take as one undoable edit. Use insert:false to keep it in media only; pause:true pauses instead (resume with record.start {resume}). */
    stop(args: { projectId?: string; jobId: string; sessionId?: string; ifRevision?: Splice.Revision; insert?: boolean; pause?: boolean; }): { jobId: string; id?: string; state: string; revision?: Splice.Revision; mediaId?: string; clipId?: string | null; output?: string; };
  };
};
