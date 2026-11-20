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
  /** "r12" or 12. */
  type Revision = number | string;
  /** A rectangle normalized to the source frame (0–1). */
  type Rect = { x: number; y: number; w: number; h: number };
  /** Curve toward the next key (agent keys default to "inOut"). A spring's duration is in frames;
   * bounce 0 is critically damped, above 0 overshoots, below 0 is overdamped. */
  type Ease =
    | "linear" | "hold" | "in" | "out" | "inOut" | "back" | "elastic" | "bounce"
    | [number, number, number, number]
    | { steps: number }
    | { spring: { duration: number; bounce?: number } };
  /** A keyframe; frames are relative to the owner's start. */
  type Key = { at: number; value: number; ease?: Ease };

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
      /** default: anchored items of a deleted clip cascade */
      connected?: "reparent" | "delete" | "trim";
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
      /** wraps and shrinks to fit its box; emoji kept */
      text?: string;
      /** lan_… */
      lane?: string;
      /** the text box in normalized canvas coordinates (layout effect), inside the canvas; fit shrink (default) keeps the style size and shrinks to fit, fill grows to the box; wrap true by default */
      box?: { x?: number; y?: number; w?: number; h?: number; fit?: "shrink" | "fill"; wrap?: boolean };
      /** text effect parameters */
      style?: { font?: string; weight?: "regular" | "medium" | "semibold" | "bold" | "heavy" | "black"; size?: number; color?: string; stroke?: string; strokeWidth?: number; shadow?: number; box?: "none" | "solid" | "rounded"; boxColor?: string; align?: "left" | "center" | "right"; lineHeight?: number; position?: "top" | "middle" | "bottom" | "lowerThird" };
      /** animate effect: how units come in; character units are grapheme clusters (emoji stay whole) */
      in?: { preset?: "none" | "fade" | "rise" | "pop" | "typewriter" | "highlight" | "bounce" | "slide"; unit?: "word" | "character" | "line"; duration?: number; stagger?: number };
      /** preset fade, sink, shrink or slide over the last duration frames (1–120, default 8); transform keys */
      out?: { preset: "fade" | "sink" | "shrink" | "slide"; duration?: number; intensity?: number };
      /** preset pulse, float, wiggle or shake repeating every period frames (6–600, default 60) until out; intensity 0–1, default 0.5; transform keys */
      loop?: { preset: "pulse" | "float" | "wiggle" | "shake"; period?: number; intensity?: number };
    }
    /** Edit Title (edit, 0.1). */
    interface TitleSet extends Base {
      cmd: "title.set";
      /** ovl_… title */
      item: string;
      /** text, or any of box, style, in, out, loop */
      text?: string;
      /** merged into the text box */
      box?: { x?: number; y?: number; w?: number; h?: number; fit?: "shrink" | "fill"; wrap?: boolean };
      /** text effect parameters, merged */
      style?: { font?: string; weight?: "regular" | "medium" | "semibold" | "bold" | "heavy" | "black"; size?: number; color?: string; stroke?: string; strokeWidth?: number; shadow?: number; box?: "none" | "solid" | "rounded"; boxColor?: string; align?: "left" | "center" | "right"; lineHeight?: number; position?: "top" | "middle" | "bottom" | "lowerThird" };
      /** animate effect, merged */
      in?: { preset?: "none" | "fade" | "rise" | "pop" | "typewriter" | "highlight" | "bounce" | "slide"; unit?: "word" | "character" | "line"; duration?: number; stagger?: number };
      /** replaces the exit keys */
      out?: { preset: "fade" | "sink" | "shrink" | "slide"; duration?: number; intensity?: number };
      /** replaces the loop keys */
      loop?: { preset: "pulse" | "float" | "wiggle" | "shake"; period?: number; intensity?: number };
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
      /** {amount,attack,release} · with items only (refused with clip) · amount 0–1, attack and release in frames; lowers sound items under speech, or null */
      duck?: { amount: number; attack?: number; release?: number } | null;
      /** number · with items only (refused with clip) · LUFS target -40…-5 (e.g. -14 for social), from the measured loudness; a target needing more than 2× gain is OUT_OF_RANGE with the loudest reachable one */
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
      /** clp_… screen take with recorded events */
      clip: string;
      /** which keystrokes show as chips: every key (typed text included), only keys pressed with ⌘, ⌃ or ⌥, or none (the keys component off). New takes start with none */
      show?: "all" | "shortcuts" | "none";
      /** 0.5–3 · chip scale */
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
      /** default: items on a partly cut clip move to the nearest surviving clip; a wholly cut clip takes its items */
      connected?: "reparent" | "delete" | "trim";
    }
    /** Delete Words (destructive, 0.2). */
    interface CutWords extends Base {
      cmd: "cut.words";
      /** word id[] */
      words: string[];
      /** string from transcript.get */
      transcriptHash?: string;
      /** as cut.range */
      connected?: "reparent" | "delete" | "trim";
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
      /** [{at, x, y, w, h, ease}] · owner frames; crop normalized to the source; ease as param.keys (commands.describe {id:"param.keys"} lists eases) */
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
      /** timeline frame the mask appears (omit: the item start) */
      from?: At;
      /** timeline frame it disappears, exclusive (omit: the item end) */
      to?: At;
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
      /** timeline frame the mask appears */
      from?: At;
      /** timeline frame it disappears, exclusive */
      to?: At;
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
      /** pauses at least this long are cut · @default 0.7 s (activity: 2 s) */
      min?: number;
      /** silence kept on each side · @default 4 (activity: 0.25 s) */
      keep?: number;
      /** auto measures against transcript words when the timeline has any, else the takes' recorded activity · @default auto */
      basis?: "auto" | "transcript" | "activity";
      /** [{media, from, to}] · busy source frames per take; splice mcp reads them from the takes' events when the basis needs them */
      activity?: { media: string; from: number; to: number }[];
    }
    /** Tighten Pace (edit, 0.3). */
    interface IntentPace extends Base {
      cmd: "intent.pace";
      /** longest pause left · @default 0.4 s (activity: 1 s) */
      maxPause?: number;
      /** auto measures against transcript words when the timeline has any, else the takes' recorded activity · @default auto */
      basis?: "auto" | "transcript" | "activity";
      /** [{media, from, to}] · busy source frames per take; splice mcp reads them from the takes' events when the basis needs them */
      activity?: { media: string; from: number; to: number }[];
    }
    /** Fit to Duration (edit, 0.3). */
    interface IntentFitDuration extends Base {
      cmd: "intent.fitDuration";
      /** number · target length; pauses go first, then fillers, then short sentences (activity: idle, then the shortest bursts) */
      seconds: number;
      /** auto measures against transcript words when the timeline has any, else the takes' recorded activity · @default auto */
      basis?: "auto" | "transcript" | "activity";
      /** [{media, from, to}] · busy source frames per take; splice mcp reads them from the takes' events when the basis needs them */
      activity?: { media: string; from: number; to: number }[];
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
      /** [{at, value, ease}] · frames relative to the owner's start (absolute for lanes and the project); ease inOut by default, or linear, hold, in, out, back, elastic, bounce, [x1,y1,x2,y2], {steps:n}, {spring:{duration, bounce}} (the result reports settle frames) */
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
      shape: "solid" | "gradient" | "rect" | "ellipse" | "arrow" | "line" | "callout" | "ring" | "heart";
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
    /** Declare Pointer (edit, 0.2). */
    interface TakePointer extends Base {
      cmd: "take.pointer";
      /** clp_… of a screen take */
      clip: string;
      /** [{at, x, y, action}] · timeline frames inside the clip; x, y in 0–1 of the captured source; action move | click. Where an automation moved or clicked unseen: the cursor travels there on a human path */
      marks: { at: number; x: number; y: number; action: string }[];
    }
    /** Trim to Steps (edit, 0.3). */
    interface IntentSteps extends Base {
      cmd: "intent.steps";
      /** clp_… · screen take */
      item: string;
      /** [{from, to}] · busy source frames; splice mcp reads them from the take's events when omitted */
      activity?: { from: number; to: number }[];
      /** kept before each step · @default 0.4 s */
      lead?: number;
      /** kept after a step's last activity · @default 1 s */
      settle?: number;
      /** longer quiet stretches inside a step are cut · @default 1.5 s */
      maxGap?: number;
      /** a marker named so drops the attempt since the step before it · @default retake: */
      retakePrefix?: string;
    }
    /** Speed Up Typing (edit, 0.3). */
    interface IntentSpeedTyping extends Base {
      cmd: "intent.speedTyping";
      /** clp_… · screen take */
      item: string;
      /** [{from, to}] · source frames; splice mcp reads them from the take's events when omitted */
      typing?: { from: number; to: number }[];
      /** 1.25–16 · rate while typing · @default 3 */
      factor?: number;
      /** shorter typing runs keep their speed · @default 1 s */
      minRun?: number;
    }
    /** Add Caption (edit, 0.3). */
    interface CaptionAdd extends Base {
      cmd: "caption.add";
      /** the cue, shown as typed (no transcript needed) */
      text: string;
      /** first timeline frame */
      from: number;
      /** timeline frame after the cue */
      to: number;
      /** lan_… · captions lane (default: the first, created if missing) */
      lane?: string;
    }
    /** Arrange (edit, 0.3). */
    interface ItemOrder extends Base {
      cmd: "item.order";
      /** ovl_… · overlay, title, shape or caption */
      item: string;
      /** one paint order across kinds, higher z in front */
      to: "front" | "back" | { above: string } | { below: string } | number;
    }
    /** Motion Preset (edit, 0.3). */
    interface MotionPreset extends Base {
      cmd: "motion.preset";
      /** clp_… | ovl_…[] · clips, overlays, titles or shapes */
      items: string[];
      /** ordinary transform keys you can edit */
      preset: "bounceIn" | "shake" | "pulse" | "float" | "wiggle" | "pop";
      /** from each item's start · @default 0 */
      at?: number;
      /** at least 2, within the item; bounceIn 24, pop 14, shake 18, loops fill the item */
      length?: number;
      /** 0–1 · @default 0.5 */
      intensity?: number;
    }
    /** Add HTML (edit, 0.3). */
    interface HtmlAdd extends Base {
      cmd: "html.add";
      /** a page (HTML, CSS, JS) drawn over a transparent canvas-sized viewport; pass html or file */
      html?: string;
      /** path to an .html file; relative URLs in it resolve next to it */
      file?: string;
      /** frame or anchor form */
      at: At;
      /** frames (1–18000) */
      length: number;
      /** full (default)|card|corner-tl|corner-tr|corner-bl|corner-br */
      placement?: string;
      /** lan_… */
      lane?: string;
      /** the rendered movie · splice mcp fills it: the page runs on a virtual clock (Date.now, performance.now, requestAnimationFrame, CSS and Web Animations follow the frame; window.seek(seconds) if defined) and must draw the same frame twice */
      movie?: string;
    }
    /** Add Music (edit, 0.3). */
    interface MusicAdd extends Base {
      cmd: "music.add";
      /** catalog ID from commands.describe {music:{query}}; edit.apply fills path, seconds, loudness, title, artist and license from the music library */
      track?: string;
      /** where the music starts · @default 0 */
      at?: At;
      /** timeline and loop repeat a short track to the timeline end; every mode cuts at the end with a fade-out; trim never repeats · @default timeline */
      fit?: "timeline" | "loop" | "trim";
      /** lower under speech (amount 0.25, attack 8, release 15 frames) · @default true */
      duck?: boolean;
      /** 0–2 · linear gain instead of normalizing */
      volume?: number;
      /** number · LUFS target (social) from the measured loudness · @default -14 */
      normalize?: number;
      /** fade where the timeline end cuts the music · @default 2 s */
      fadeOut?: number;
      /** lan_… · audio lane (default: a free audio lane, created if needed) */
      lane?: string;
      /** add a track whose license is unknown · @default false */
      allowUnlicensed?: boolean;
      /** filled from the library (or pass a file instead of track) */
      path?: string;
      /** number · filled: length, used where no media backend probes the file */
      seconds?: number;
      /** number · filled: integrated LUFS from the catalog */
      loudness?: number;
      /** filled: credit */
      title?: string;
      /** filled: credit */
      artist?: string;
      /** filled: credit; unknown is refused without allowUnlicensed */
      license?: string;
    }
    /** Add Counter (edit, 0.3). */
    interface CounterAdd extends Base {
      cmd: "counter.add";
      at: At;
      /** the count runs over all of it · @default 90 */
      length?: number;
      /** number · @default 0 */
      from?: number;
      /** number · the value it lands on */
      to: number;
      /** text with {n} · e.g. {n}%, $ {n}, {n}k users · @default {n} */
      format?: string;
      /** int 0–4 · @default 0 */
      decimals?: number;
      /** thousands separator · @default true */
      group?: boolean;
      /** ease · inOut, linear, {spring:{duration, bounce}}, … · @default out */
      ease?: Ease;
      /** lan_… · graphics lane */
      lane?: string;
      /** text box, as title.add */
      box?: { x?: number; y?: number; w?: number; h?: number; fit?: "shrink" | "fill"; wrap?: boolean };
      /** text effect parameters, as title.add */
      style?: { font?: string; weight?: "regular" | "medium" | "semibold" | "bold" | "heavy" | "black"; size?: number; color?: string; stroke?: string; strokeWidth?: number; shadow?: number; box?: "none" | "solid" | "rounded"; boxColor?: string; align?: "left" | "center" | "right"; lineHeight?: number; position?: "top" | "middle" | "bottom" | "lowerThird" };
    }
    /** Add Progress (edit, 0.3). */
    interface ProgressAdd extends Base {
      cmd: "progress.add";
      /** @default bar */
      shape?: "bar" | "ring";
      /** @default 0 */
      at?: At;
      /** "timeline" runs to the end · @default timeline */
      length?: "timeline" | number;
      /** {x, y, w, h} · normalized canvas; a thin strip at the bottom, or a centered circle */
      rect?: Rect;
      /** #rrggbb · the part done */
      fill?: string;
      /** #rrggbb · the part to go */
      track?: string;
      /** 0–1 · @default 0.25 */
      trackOpacity?: number;
      /** 0–0.5 · bar corners */
      corner?: number;
      /** 0–0.05 · ring thickness */
      strokeWidth?: number;
      /** lan_… · graphics lane */
      lane?: string;
    }
    /** Add Particles (edit, 0.3). */
    interface ParticlesAdd extends Base {
      cmd: "particles.add";
      at: At;
      /** @default 90 */
      length?: number;
      /** @default confetti */
      preset?: "confetti" | "sparkle" | "snow" | "hearts";
      /** {x, y, w, h} · normalized canvas region = the canvas */
      rect?: Rect;
      /** int 1–120 · two shapes cannot exceed 256 layers in a frame · @default 60 */
      count?: number;
      /** int 0–1000000 · same seed, same particles · @default 1 */
      seed?: number;
      /** 0.002–0.08 · of the canvas's shorter side · @default 0.014 */
      size?: number;
      /** lan_… · graphics lane */
      lane?: string;
    }
    /** Apply Look (edit, 0.3). */
    interface LookApply extends Base {
      cmd: "look.apply";
      /** clp_… | ovl_… | lan_… | project */
      target: string;
      /** ordinary color, window and captions effects; parts that do not apply to the target are skipped and named */
      pack?: "vivid" | "film" | "mono" | "pastel" | "neon";
      /** a look.save .json file instead of a pack (relative paths resolve in the package); its window, camera, cursor, color and captions effects are written as saved */
      file?: string;
      /** the look file's effects by type, filled from file by splice mcp and splice apply */
      effects?: Record<string, unknown>;
    }
    /** Save Look (edit, 0.3). */
    interface LookSave extends Base {
      cmd: "look.save";
      /** clp_… | ovl_… | lan_… | project */
      target: string;
      /** .json path (relative paths resolve in the package) · written when the batch commits, never on a dry run; changes nothing in the project */
      file: string;
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
      background?: "gradient" | "color" | "none" | "image" | "wallpaper";
      /** Color · #rrggbb · @default "#5b6f63" */
      color?: string;
      /** Color 2 · #rrggbb · @default "#1d2420" */
      color2?: string;
      /** Gradient angle (degrees; 0 runs left to right, 90 top to bottom; unset runs corner to corner) · 0–360 */
      angle?: number;
      /** Background image (a file path, imported on apply, or its med_ ID) */
      image?: string;
      /** Image blur · 0–1 · @default 0 */
      blur?: number;
      /** Wallpaper (generated gradient) · @default "aurora" */
      wallpaper?: "aurora" | "sunset" | "ocean" | "forest" | "ember" | "slate";
    };
    /** Cursor: screen takes. */
    cursor: {
      /** Size · 0.5–4 · @default 1.5 */
      size?: number;
      /** Click rings · @default true */
      clickRings?: boolean;
      /** Hide cursor · @default false */
      hidden?: boolean;
      /** Hide when idle · @default true */
      hideWhenIdle?: boolean;
      /** Smoothing · 0–1 · @default 1 */
      smoothing?: number;
      /** Click pulse · @default true */
      clickPulse?: boolean;
      /** Path · @default "auto" */
      path?: "auto" | "recorded";
      /** Path arc · 0–0.5 · @default 0.15 */
      arc?: number;
      /** Arrive before click (s) · 0–1 · @default 0.15 */
      arriveBefore?: number;
    };
    /** Camera Layout: screen takes. */
    camera: {
      /** Corner · @default "br" */
      corner?: "br" | "bl" | "tr" | "tl";
      /** Size · 0.1–1 · @default 0.24 */
      size?: number | { keys: Key[] };
      /** Roundness · 0–1 · @default 0.2 */
      roundness?: number | { keys: Key[] };
      /** Shadow · 0–1 · @default 0.5 */
      shadow?: number | { keys: Key[] };
      /** Background · @default "keep" */
      background?: "keep" | "blur" | "remove";
      /** Aspect · @default "square" */
      aspect?: "square" | "circle" | "4:3" | "3:4" | "16:9" | "9:16";
      /** Center x (0–1 of the canvas; with y, replaces the corner) · 0–1 */
      x?: number | { keys: Key[] };
      /** Center y (0–1 of the canvas; with x, replaces the corner) · 0–1 */
      y?: number | { keys: Key[] };
      /** Mirror · @default false */
      mirror?: boolean;
      /** Shrink when zoomed · @default false */
      shrinkWhenZoomed?: boolean;
      /** Layout (keys switch at their frame and ease over 0.3 s) · @default "bubble" */
      layout?: "bubble" | "full" | "hidden" | { keys: { at: number; value: "bubble" | "full" | "hidden"; ease?: Ease }[] };
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
      /** Show · @default "all" */
      show?: "all" | "shortcuts";
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
      /** First frame (relative to the owner's start; unset: from its start) · 0–10000000 */
      start?: number;
      /** Frame after the last (relative to the owner's start; unset: to its end) · 0–10000000 */
      end?: number;
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
      kind?: "dissolve" | "dip" | "wipe" | "push" | "slide" | "zoom" | "whip" | "zoomPunch" | "swipe";
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
    /** Text Box: titles, graphics lanes and the project. */
    layout: {
      /** X · 0–0.98 · @default 0.06 */
      x?: number;
      /** Y · 0–0.98 · @default 0.07 */
      y?: number;
      /** Width · 0.02–1 · @default 0.88 */
      w?: number;
      /** Height · 0.02–1 · @default 0.12 */
      h?: number;
      /** Fit · @default "shrink" */
      fit?: "shrink" | "fill";
      /** Wrap · @default true */
      wrap?: boolean;
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
    /** Counter: titles (counter.add). */
    counter: {
      /** Value · -1000000000–1000000000 · @default 0 */
      value?: number | { keys: Key[] };
      /** Format · @default "{n}" */
      format?: string;
      /** Decimals · 0–4 · @default 0 */
      decimals?: number;
      /** Thousands separator · @default true */
      group?: boolean;
    };
    /** Progress: rect and ring shapes (progress.add). */
    progress: {
      /** Progress · 0–1 · @default 0 */
      value?: number | { keys: Key[] };
      /** Track · #rrggbb · @default "#ffffff" */
      track?: string;
      /** Track opacity · 0–1 · @default 0.25 */
      trackOpacity?: number;
    };
    /** Particles: shapes (particles.add). */
    particles: {
      /** Preset · @default "confetti" */
      preset?: "confetti" | "sparkle" | "snow" | "hearts";
      /** Count · 1–120 · @default 60 */
      count?: number;
      /** Seed · 0–1000000 · @default 1 */
      seed?: number;
      /** Size · 2e-3–0.08 · @default 0.014 */
      size?: number;
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
    | op.ZoomRemove
    | op.TakePointer
    | op.IntentSteps
    | op.IntentSpeedTyping
    | op.CaptionAdd
    | op.ItemOrder
    | op.MotionPreset
    | op.HtmlAdd
    | op.MusicAdd
    | op.CounterAdd
    | op.ProgressAdd
    | op.ParticlesAdd
    | op.LookApply
    | op.LookSave;
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
      order(args: Omit<Splice.op.ItemOrder, "cmd">): Splice.op.ItemOrder;
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
      add(args: Omit<Splice.op.CaptionAdd, "cmd">): Splice.op.CaptionAdd;
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
      steps(args: Omit<Splice.op.IntentSteps, "cmd">): Splice.op.IntentSteps;
      speedTyping(args: Omit<Splice.op.IntentSpeedTyping, "cmd">): Splice.op.IntentSpeedTyping;
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
    take: {
      pointer(args: Omit<Splice.op.TakePointer, "cmd">): Splice.op.TakePointer;
    };
    motion: {
      preset(args: Omit<Splice.op.MotionPreset, "cmd">): Splice.op.MotionPreset;
    };
    html: {
      add(args: Omit<Splice.op.HtmlAdd, "cmd">): Splice.op.HtmlAdd;
    };
    music: {
      add(args: Omit<Splice.op.MusicAdd, "cmd">): Splice.op.MusicAdd;
    };
    counter: {
      add(args: Omit<Splice.op.CounterAdd, "cmd">): Splice.op.CounterAdd;
    };
    progress: {
      add(args: Omit<Splice.op.ProgressAdd, "cmd">): Splice.op.ProgressAdd;
    };
    particles: {
      add(args: Omit<Splice.op.ParticlesAdd, "cmd">): Splice.op.ParticlesAdd;
    };
    look: {
      apply(args: Omit<Splice.op.LookApply, "cmd">): Splice.op.LookApply;
      save(args: Omit<Splice.op.LookSave, "cmd">): Splice.op.LookSave;
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
    inspect(args?: { projectId?: string; ids?: string[]; items?: string[]; }): { revision: Splice.Revision; items: unknown[]; };
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
    /** Analyze a clip's media in timeline frames. Kinds: loudness and pauses (default); activity of a screen take (idle, typing, clicks, scroll and busy ranges from its recorded input and transcript, never the typed characters); peaks, beats and sound labels (speech, music, laughter); shot cuts, on-screen text with boxes, faces and people every second; matte builds the person matte for a cutout effect. Results are cached per file. Native kinds need macOS. */
    analyze(args: { projectId?: string; clip: string; kinds?: ("loudness" | "pauses" | "activity" | "peaks" | "beats" | "sounds" | "shots" | "text" | "faces" | "people" | "matte")[]; async?: boolean; }): { revision: Splice.Revision; clip: string; components?: unknown[]; pauses?: unknown[]; activity?: Record<string, unknown>; peaks?: Record<string, unknown>; beats?: unknown[]; sounds?: unknown[]; shots?: unknown[]; text?: unknown[]; faces?: unknown[]; people?: unknown[]; matte?: string; };
  };
  view: {
    /** Render PNG frames from the export composition. Pass explicit frame numbers or a filmstrip count; sheet:true returns one contact-sheet image of them all, and strip:true adds a heatmap of what changed between each consecutive pair after them. macOS builds only; elsewhere it fails with UNSUPPORTED_PLATFORM. */
    frames(args?: { projectId?: string; at?: number[]; count?: number; sheet?: boolean; strip?: boolean; }): { revision: Splice.Revision; frames: unknown[]; sheet?: string; };
    /** Check a range for problems a few frames would miss. Camera jumps and pictures pushed off the canvas come from the evaluated timeline on every platform; on macOS, black or frozen spans and transitions that disturb the backdrop come from frames rendered at 5 per second, with a contact sheet of the flagged frames. Each issue has its span and a fix. */
    check(args?: { projectId?: string; from?: number; to?: number; checks?: ("camera" | "offCanvas" | "black" | "frozen" | "bleed")[]; }): { revision: Splice.Revision; issues: unknown[]; pixels: boolean; sheet?: string; };
  };
  commands: {
    /** Search the command registry. Pass an exact ID to see parameters, release tier and an example. music:{query, mood, bpm, minLength, maxLength, limit} searches the music library instead (tracks for music.add, with license, length in seconds, BPM, loudness). */
    describe(args?: { projectId?: string; id?: string; query?: string; format?: "json" | "ts"; music?: { query?: string; mood?: string; bpm?: number; minLength?: number; maxLength?: number; limit?: number }; }): { commands: unknown[]; id?: string; title?: string; group?: string; risk?: string; cut?: string; available?: boolean; planned?: boolean; params?: Record<string, unknown>; shortcut?: string | null; ui?: unknown[]; example?: Record<string, unknown>; captionModel?: string; presets?: string[]; effectTypes?: Record<string, unknown>; resolution?: string; format?: "ts"; text?: string; library?: string; tracks?: unknown[]; total?: number; moods?: string[]; note?: string; };
  };
  session: {
    /** Start an agent editing session and get its checkpoint revision and session ID. */
    start(args?: { projectId?: string; agent?: string; }): { sessionId: string; checkpoint: Splice.Revision; revision: Splice.Revision; access?: string; resumed?: boolean; };
  };
  edit: {
    /** Apply one atomic batch of registry commands. Use dryRun to preview and idempotencyKey for a safe retry. The result carries lints (warnings with a fix and a suggested ops batch: text outside the safe zone or over the window, zooms past the canvas, long static stretches) and pacing stats. */
    apply(args: { projectId?: string; sessionId?: string; ifRevision: Splice.Revision; ops: Splice.Op[]; summary?: string; dryRun?: boolean; idempotencyKey?: string; }): { revision: Splice.Revision; dryRun?: boolean; replayed?: boolean; changes: unknown[]; sideEffects: unknown[]; diff: unknown[]; lints?: unknown[]; pacing?: Record<string, unknown>; };
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
    /** Export the timeline: presets for social (youtube-1080/4k, shorts, reels, tiktok, linkedin), masters (prores-422, prores-4444 and hevc-alpha keep transparency), audio (audio-m4a, audio-wav), gif (about 15 fps, at most 150 frames and 480 px wide; a longer range is refused with the limit) and poster (done at once). from/to or marker limit the range; markers become chapters; captions:"srt" writes a sidecar; variants:[{aspect, preset}] renders several aspects, each reframed to fill; format:"fcpxml" writes the cut for Final Cut Pro (every platform). Video and audio need macOS and run as jobs (two at a time). Without sessionId the export opens its own session. */
    start(args: { projectId?: string; sessionId?: string; ifRevision: Splice.Revision; preset?: "social" | "social-h264" | "social-hevc" | "youtube-1080" | "youtube-4k" | "shorts" | "reels" | "tiktok" | "linkedin" | "prores-422" | "prores-4444" | "hevc-alpha" | "audio-m4a" | "audio-wav" | "gif" | "poster"; format?: "video" | "fcpxml"; from?: number; to?: number; marker?: string; bitrate?: number; captions?: "srt" | "vtt"; chapters?: boolean; variants?: Record<string, unknown>[]; path?: string; overwrite?: boolean; }): { jobId: string; state: string; settings?: Record<string, unknown>; output?: string; jobs?: unknown[]; };
  };
  jobs: {
    /** Check the progress and result of an export, transcription or recording job. wait (seconds, up to 120) returns once the job stops running (a recording also when it pauses), so one call (or one code.run: waiting is not counted against its 2 s) sees it done; without wait, each call advances a running export by up to 50 ms. */
    status(args: { projectId?: string; jobId: string; wait?: number; }): { jobId: string; id?: string; kind?: string; state: string; progress?: number; settings?: Record<string, unknown>; output?: string; elapsed?: number; bytes?: number; droppedFrames?: number; videoFrames?: number; stopsAt?: number; then?: string; fps?: number; eta?: number; revision?: Splice.Revision; };
    /** Cancel a running or pending job. */
    cancel(args: { projectId?: string; jobId: string; sessionId?: string; ifRevision?: Splice.Revision; }): { jobId: string; id?: string; state: string; };
  };
  record: {
    /** Start headless capture of a display, window or area. To record an app, pass source:{window:{app:"Safari"}} (app name or bundle ID; add title when it has several windows); size:[w,h] resizes that window (points) first. dryRun lists sources, microphones and cameras (stable device IDs) and permissions; window titles only with titles:true. mic and camera take true (default device) or a device ID. systemAudio: true, false or {apps:[…]} (only those apps' sound). Never sleep to time a take: duration (seconds of recorded time) ends it by itself, then:"pause" pauses there instead, and resume:jobId with duration records exactly that much more; record.stop {wait} or jobs.status {wait} returns once it ends. countdown is seconds. The receipt's settings echo what will be recorded (source size, fps, devices, canvas). create:true sizes the new canvas to the source (aspect:auto) unless aspect is given. sessionId alone joins an open session (ifRevision optional). voiceover:true records the mic onto a new audio lane at frame at; camera:true with screen:false is a camera-only take; retakeOf places the take after an earlier one; exclude hides apps, "desktop" hides desktop icons. resume with marker drops a marker. recover:true publishes takes a crash left staged. */
    start(args?: { projectId?: string; path?: string; create?: boolean; sessionId?: string; ifRevision?: Splice.Revision; source?: Record<string, unknown>; display?: string | number; window?: string | number | Record<string, unknown>; area?: Record<string, unknown>; mic?: boolean | string; systemAudio?: boolean | Record<string, unknown>; camera?: boolean | string; events?: boolean; insert?: boolean; still?: boolean; countdown?: number; dryRun?: boolean; duration?: number; then?: "stop" | "pause"; size?: number[]; aspect?: "auto" | "9:16" | "4:5" | "1:1" | "4:3" | "16:9"; voiceover?: boolean; at?: number; screen?: boolean; retakeOf?: string; exclude?: string[]; resume?: string; marker?: string; recover?: boolean; titles?: boolean; }): { jobId: string; state: string; projectId?: string; sessionId?: string; settings?: Record<string, unknown>; sources?: Record<string, unknown>; devices?: unknown[]; recovered?: unknown[]; markers?: unknown[]; permissions?: Record<string, unknown>; identity?: Record<string, unknown>; };
    /** Finish a recording (started with record.start, e.g. source:{window:{app}}) and insert the take as one undoable edit. Returns a take summary (frames, seconds, size, fps, tracks, clicks, keys, micPeakDb or silent, droppedFrames) and the lints an edit returns, including TAKE_ASPECT when the take does not fill the canvas. wait (seconds, up to 120) first lets a timed take (duration) end by itself. Use insert:false to keep it in media only; pause:true pauses instead (resume with record.start {resume}). */
    stop(args: { projectId?: string; jobId: string; sessionId?: string; ifRevision?: Splice.Revision; insert?: boolean; pause?: boolean; wait?: number; }): { jobId: string; id?: string; state: string; revision?: Splice.Revision; mediaId?: string; clipId?: string | null; output?: string; take?: Record<string, unknown>; lints?: unknown[]; pacing?: Record<string, unknown>; settings?: Record<string, unknown>; warning?: string; };
  };
};
