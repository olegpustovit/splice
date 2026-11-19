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
  /** "r12" or 12. */
  type Revision = number | string;

  namespace op {
    interface Base {
      /** Binds the result as "$name" for later ops ("$name.new": its first created item). */
      ref?: string;
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
    /** Add Scale Animation (edit, 0.2). */
    interface MotionAdd extends Base {
      cmd: "motion.add";
      /** frame or anchor form */
      at: At;
      /** @default 120 */
      length?: number;
      /** scale · @default 1 */
      from?: number;
      /** scale · @default 1.32 */
      to?: number;
    }
    /** Edit Scale Keyframes (edit, 0.2). */
    interface MotionSet extends Base {
      cmd: "motion.set";
      /** ovl_… motion */
      item: string;
      /** 0.5–2 */
      from?: number;
      /** 0.5–2 */
      to?: number;
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
      kind: "graphics" | "overlay" | "motion" | "captions";
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
      /** clp_… */
      clip: string;
      component: "mic" | "system";
      /** 0–2 */
      volume: number;
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
      type: "window" | "cursor" | "clickZoom" | "camera" | "captions" | "keystrokes";
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
      type?: string;
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
      type?: string;
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
      /** clp_…[] · takes to caption (default: every clip with a transcript) */
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
    /** Join Clips (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface ClipJoin extends Base {
      cmd: "clip.join";
    }
    /** Ungroup (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface ClipUngroup extends Base {
      cmd: "clip.ungroup";
    }
    /** Restore Cuts (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface ClipRestore extends Base {
      cmd: "clip.restore";
    }
    /** Reorder Lane (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface LaneReorder extends Base {
      cmd: "lane.reorder";
    }
    /** Animated Crop (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface CropKeys extends Base {
      cmd: "crop.keys";
    }
    /** Add Transition (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface TransitionAdd extends Base {
      cmd: "transition.add";
    }
    /** Zoom Settings (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface ZoomSet extends Base {
      cmd: "zoom.set";
    }
    /** Add Mask (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface MaskAdd extends Base {
      cmd: "mask.add";
    }
    /** Edit Mask (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface MaskSet extends Base {
      cmd: "mask.set";
    }
    /** Translate Captions (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface CaptionsTranslate extends Base {
      cmd: "captions.translate";
    }
    /** Import SRT / VTT (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface CaptionsImport extends Base {
      cmd: "captions.import";
    }
    /** Link Audio (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface AudioLink extends Base {
      cmd: "audio.link";
    }
    /** Unlink Audio (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface AudioUnlink extends Base {
      cmd: "audio.unlink";
    }
    /** Move into Sync (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface AudioSync extends Base {
      cmd: "audio.sync";
    }
    /** Style (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface StyleSet extends Base {
      cmd: "style.set";
    }
    /** Cut Pauses (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface CutPauses extends Base {
      cmd: "cut.pauses";
    }
    /** Tighten Pace (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface IntentPace extends Base {
      cmd: "intent.pace";
    }
    /** Fit to Duration (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface IntentFitDuration extends Base {
      cmd: "intent.fitDuration";
    }
    /** Keep Shots (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface IntentKeepShots extends Base {
      cmd: "intent.keepShots";
    }
    /** Auto Zoom (edit, 0.3, planned: fails with NOT_IMPLEMENTED). */
    interface IntentAutoZoom extends Base {
      cmd: "intent.autoZoom";
    }
  }
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
    | op.MotionAdd
    | op.MotionSet
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
    | op.ZoomSet
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
    | op.IntentAutoZoom;
}

declare const splice: {
  /** The projectId and sessionId passed to code.run. Tool calls default to them. */
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
    };
    clip: {
      speed(args: Omit<Splice.op.ClipSpeed, "cmd">): Splice.op.ClipSpeed;
      insert(args: Omit<Splice.op.ClipInsert, "cmd">): Splice.op.ClipInsert;
      lift(args: Omit<Splice.op.ClipLift, "cmd">): Splice.op.ClipLift;
      crop(args: Omit<Splice.op.ClipCrop, "cmd">): Splice.op.ClipCrop;
      join(args: Omit<Splice.op.ClipJoin, "cmd">): Splice.op.ClipJoin;
      ungroup(args: Omit<Splice.op.ClipUngroup, "cmd">): Splice.op.ClipUngroup;
      restore(args: Omit<Splice.op.ClipRestore, "cmd">): Splice.op.ClipRestore;
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
    motion: {
      add(args: Omit<Splice.op.MotionAdd, "cmd">): Splice.op.MotionAdd;
      set(args: Omit<Splice.op.MotionSet, "cmd">): Splice.op.MotionSet;
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
    };
    zoom: {
      set(args: Omit<Splice.op.ZoomSet, "cmd">): Splice.op.ZoomSet;
    };
    mask: {
      add(args: Omit<Splice.op.MaskAdd, "cmd">): Splice.op.MaskAdd;
      set(args: Omit<Splice.op.MaskSet, "cmd">): Splice.op.MaskSet;
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
  };
  project: {
    /** Open a .splice project by path or inspect an open project by ID. Use create:true to make a new project. */
    info(args?: { projectId?: string; path?: string; create?: boolean; }): { projectId: string; revision: Splice.Revision; fps: number; size?: unknown[]; aspect?: string; durationFrames: number; access: string; lanes: unknown[]; media: unknown[]; offlineMedia?: unknown[]; transcript?: Record<string, unknown>; effects?: Record<string, unknown>; openSession?: string | null; };
  };
  timeline: {
    /** List the spine, drawing lanes and markers with resolved frame ranges. Use detail:full for all item fields. */
    snapshot(args?: { projectId?: string; from?: number; to?: number; detail?: "concise" | "full"; limit?: number; cursor?: string; }): { revision: Splice.Revision; fps?: number; range?: Record<string, unknown>; lanes: unknown[]; spine: unknown[]; markers: unknown[]; total?: number; nextCursor?: string | null; };
    /** Find items, markers, words and edit points by text, kind, lane, color or frame range. */
    find(args?: { projectId?: string; text?: string; kind?: string; lane?: string; color?: string; from?: number; to?: number; limit?: number; }): { revision: Splice.Revision; matches: unknown[]; total: number; };
  };
  items: {
    /** Inspect up to 50 objects by ID, including anchors, resolved frames, children and inherited effects. */
    inspect(args: { projectId?: string; ids: string[]; }): { revision: Splice.Revision; items: unknown[]; };
  };
  takes: {
    /** Read the take's cursor, clicks and keys in timeline frames, with keystrokes redacted by project policy. */
    events(args: { projectId?: string; clip: string; from?: number; to?: number; }): { revision: Splice.Revision; clip: string; cursor?: unknown[]; clicks: unknown[]; keys: unknown[]; };
  };
  transcript: {
    /** Page through words by ID or get caption cues as JSON, SRT or WebVTT. Use the returned hash with word anchors. */
    get(args?: { projectId?: string; clip?: string; cursor?: string; limit?: number; captions?: "json" | "srt" | "vtt"; lane?: string; }): { revision: Splice.Revision; hash: string | null; clip?: string; words: unknown[]; fillers?: string[]; nextCursor?: string | null; lanes?: unknown[]; cues?: unknown[] | number; format?: "srt" | "vtt"; text?: string; };
  };
  audio: {
    /** Read loudness, peaks and silence or pause ranges for a clip's audio components. */
    summary(args: { projectId?: string; clip: string; }): { revision: Splice.Revision; clip: string; components: unknown[]; silences: unknown[]; };
  };
  view: {
    /** Render PNG frames from the export composition. Pass explicit frame numbers or a filmstrip count. */
    frames(args?: { projectId?: string; at?: number[]; count?: number; }): { revision: Splice.Revision; frames: unknown[]; };
  };
  commands: {
    /** Search the command registry. Pass an exact ID to see parameters, release tier and an example. */
    describe(args?: { projectId?: string; id?: string; query?: string; format?: "json" | "ts"; }): { commands: unknown[]; id?: string; title?: string; group?: string; risk?: string; cut?: string; planned?: boolean; params?: Record<string, unknown>; shortcut?: string | null; ui?: unknown[]; example?: Record<string, unknown>; captionModel?: string; presets?: string[]; effectTypes?: Record<string, unknown>; resolution?: string; format?: "ts"; text?: string; };
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
    /** Start an MP4 export job. Use overwrite:true to replace an existing output file. */
    start(args: { projectId?: string; sessionId?: string; ifRevision: Splice.Revision; preset?: string; path?: string; overwrite?: boolean; }): { jobId: string; state: string; settings?: Record<string, unknown>; };
  };
  jobs: {
    /** Check the progress and result of an export, transcription or recording job. */
    status(args: { projectId?: string; jobId: string; }): { jobId: string; id?: string; kind?: string; state: string; progress?: number; settings?: Record<string, unknown>; output?: string; elapsed?: number; bytes?: number; droppedFrames?: number; revision?: Splice.Revision; };
    /** Cancel a running or pending job. */
    cancel(args: { projectId?: string; jobId: string; sessionId?: string; ifRevision?: Splice.Revision; }): { jobId: string; id?: string; state: string; };
  };
};
