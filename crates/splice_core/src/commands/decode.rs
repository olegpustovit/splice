use super::{CommandError as Error, DecodeScratch, Json, args::*, op::*};
use splice_model::{CameraMode, Duck, Ease, Fit, Frame, Placement, Rect, ShapeKind, Spring, Zoom};

type DecodedOp = Result<Op, Error>;

pub(super) fn item_split(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ItemSplit {
        item: a.target("item")?,
        at: a.frame("at")?,
    })
}

pub(super) fn item_trim(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let edge = match a.string("edge")? {
        "in" => TrimEdge::In,
        "out" => TrimEdge::Out,
        _ => return Err(Error::bad("edge", "in or out")),
    };
    Ok(Op::ItemTrim {
        item: a.target("item")?,
        edge,
        position: position(a)?,
        ripple: a.flag("ripple", true)?,
    })
}

pub(super) fn item_move(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    targets_into(a.required("items")?, "items", &mut scratch.targets)?;
    let position = position(a)?;
    let carry_overlays = a.flag("carryOverlays", true)?;
    let ripple = a.flag("ripple", true)?;
    Ok(Op::ItemMove {
        items: std::mem::take(&mut scratch.targets),
        position,
        carry_overlays,
        ripple,
    })
}

pub(super) fn item_delete(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let keep_overlays = match a.opt_string("overlays")?.as_deref() {
        None | Some("delete") => false,
        Some("keep") => true,
        _ => return Err(Error::bad("overlays", "delete or keep")),
    };
    Ok(Op::ItemDelete {
        items: a.targets("items")?,
        ripple: a.flag("ripple", true)?,
        keep_overlays,
        connected: connected(a)?,
    })
}

fn connected(a: &Json) -> Result<Option<Connected>, Error> {
    Ok(match a.opt_string("connected")?.as_deref() {
        None => None,
        Some("reparent") => Some(Connected::Reparent),
        Some("delete") => Some(Connected::Delete),
        Some("trim") => Some(Connected::Trim),
        Some(_) => return Err(Error::bad("connected", "reparent, delete or trim")),
    })
}

pub(super) fn item_duplicate(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ItemDuplicate {
        items: a.targets("items")?,
        at: a.opt_frame("at")?,
    })
}

pub(super) fn item_set(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    let color = a.string_patch("color")?;
    if let Some(Some(color)) = color
        && !["red", "orange", "yellow", "green", "blue", "purple", "gray"].contains(&color)
    {
        return Err(Error::bad(
            "color",
            "red, orange, yellow, green, blue, purple, gray, or null",
        ));
    }
    targets_into(a.required("items")?, "items", &mut scratch.targets)?;
    let name = a.string_patch("name")?;
    let fade_in = a.opt_int("fadeIn")?;
    let fade_out = a.opt_int("fadeOut")?;
    Ok(Op::ItemSet {
        items: std::mem::take(&mut scratch.targets),
        name: name.map(|name| name.map(|name| reuse_string(&mut scratch.name, name))),
        color: color.map(|color| color.map(|color| reuse_string(&mut scratch.color, color))),
        fade_in,
        fade_out,
    })
}

fn reuse_string(scratch: &mut String, value: &str) -> String {
    scratch.clear();
    scratch.push_str(value);
    std::mem::take(scratch)
}

pub(super) fn clip_speed(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let speed = a.number("speed")?;
    if !(0.25..=16.0).contains(&speed) {
        return Err(Error::bad("speed", "a number from 0.25 to 16"));
    }
    Ok(Op::ClipSpeed {
        items: a.targets("items")?,
        speed,
        ripple: a.flag("ripple", true)?,
    })
}

pub(super) fn clip_insert(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let insert = a.flag("insert", true)?;
    let source = match (a.opt_target("media")?, a.opt_string("path")?) {
        (Some(id), None) if insert => InsertSource::Media(id),
        (None, Some(path)) if !path.is_empty() && !path.contains('\0') => InsertSource::Path(path),
        _ => {
            return Err(Error::bad(
                "media or path",
                "exactly one media ID or nonempty file path",
            ));
        }
    };
    Ok(Op::ClipInsert {
        source,
        at: a.opt_frame("at")?,
        ripple: a.flag("ripple", true)?,
        insert,
        lane: a.opt_target("lane")?,
    })
}

pub(super) fn clip_lift(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ClipLift {
        items: a.targets("items")?,
        lane: a.opt_target("lane")?,
    })
}

pub(super) fn overlay_replace_spine(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::OverlayReplaceSpine {
        item: a.target("item")?,
        target: a.opt_target("target")?,
    })
}

pub(super) fn overlay_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let content = match a.string("kind")? {
        "title" => OverlayContent::Title(a.opt_string("text")?),
        "zoom" => OverlayContent::Zoom(zoom_args(a)?),
        "media" => OverlayContent::Media {
            media: a.target("media")?,
            source: a.opt_int("source")?.unwrap_or(0),
            placement: a
                .opt_string("placement")?
                .as_deref()
                .map(placement)
                .transpose()?
                .unwrap_or(Placement::Card),
            behind_presenter: a.flag("behindPresenter", false)?,
            muted: a.flag("muted", true)?,
            crop: crop(a)?.flatten(),
            fit: a
                .opt_string("fit")?
                .as_deref()
                .map(fit)
                .transpose()?
                .unwrap_or(Fit::Fill),
        },
        _ => return Err(Error::bad("kind", "title, zoom or media")),
    };
    Ok(Op::OverlayAdd {
        content,
        at: a.opt_frame("at")?,
        anchor: a.field("anchor").map(anchor).transpose()?,
        lane: a.opt_target("lane")?,
        length: a.opt_int("length")?,
    })
}

pub(super) fn html_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let movie = match a.opt_string("movie")? {
        Some(movie) if !movie.is_empty() && !movie.contains('\0') => movie,
        _ => {
            return Err(Error::rule(
                "UNSUPPORTED_PLATFORM",
                "movie",
                None,
                "the page's rendered movie",
                "Run html.add through splice mcp or splice apply on macOS: they render html or file into movie with WebKit.",
            ));
        }
    };
    let length = a.opt_int("length")?.unwrap_or(0);
    if !(1..=30 * 60 * 10).contains(&length) {
        return Err(Error::bad(
            "length",
            "1–18000 frames (10 minutes at 30 fps)",
        ));
    }
    Ok(Op::HtmlAdd {
        movie,
        at: a.frame("at")?,
        length,
        placement: a
            .opt_string("placement")?
            .as_deref()
            .map(placement)
            .transpose()?
            .unwrap_or(Placement::Full),
        lane: a.opt_target("lane")?,
    })
}

pub(super) fn title_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::TitleAdd {
        at: a.frame("at")?,
        length: a.opt_int("length")?.unwrap_or(90),
        text: a.opt_string("text")?,
        lane: a.opt_target("lane")?,
        look: super::text::title_look(a)?,
    })
}

pub(super) fn title_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let text = a.opt_string("text")?;
    let look = super::text::title_look(a)?;
    if text.is_none() && look.is_none() {
        return Err(Error::bad(
            "text",
            "a string (or pass box, style, in, out or loop)",
        ));
    }
    Ok(Op::TitleSet {
        item: a.target("item")?,
        text,
        look,
    })
}

/// A camera segment's framing: `amount` (1–5, default 1.5), `focus`, `mode`, `spring`.
fn zoom_args(a: &Json) -> Result<Zoom, Error> {
    Ok(Zoom {
        amount: zoom_amount(a)?.unwrap_or(1.5),
        focus: focus(a)?.flatten(),
        mode: zoom_mode(a)?.unwrap_or_default(),
        spring: zoom_spring(a)?.unwrap_or_default(),
    })
}

fn zoom_amount(a: &Json) -> Result<Option<f64>, Error> {
    let [low, high] = Zoom::AMOUNT;
    a.opt_number("amount")?
        .map(|amount| {
            if (low..=high).contains(&amount) {
                Ok(amount)
            } else {
                Err(Error::bad(
                    "amount",
                    "a magnification from 1 to 5 (1.5 is a typical push-in)",
                ))
            }
        })
        .transpose()
}

fn zoom_mode(a: &Json) -> Result<Option<CameraMode>, Error> {
    a.opt_string("mode")?
        .map(|name| {
            CameraMode::parse(&name).ok_or_else(|| Error::bad("mode", "auto, content or frame"))
        })
        .transpose()
}

fn zoom_spring(a: &Json) -> Result<Option<Spring>, Error> {
    a.opt_string("spring")?
        .map(|name| {
            Spring::parse(&name)
                .ok_or_else(|| Error::bad("spring", "focused, smooth, snappy or instant"))
        })
        .transpose()
}

pub(super) fn zoom_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ZoomAdd {
        at: a.frame("at")?,
        length: a.opt_int("length")?.unwrap_or(90),
        zoom: zoom_args(a)?,
        lane: a.opt_target("lane")?,
    })
}

pub(super) fn zoom_remove(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ZoomRemove {
        zoom: a.target("zoom")?,
    })
}

pub(super) fn zoom_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ZoomEdit {
        zoom: a.target("zoom")?,
        amount: zoom_amount(a)?,
        focus: focus(a)?,
        mode: zoom_mode(a)?,
        spring: zoom_spring(a)?,
    })
}

/// `focus`: absent (None), null (Some(None)) or a normalized canvas point {x, y}.
fn focus(a: &Json) -> Result<Option<Option<[f64; 2]>>, Error> {
    let Some(value) = a.field("focus") else {
        return Ok(None);
    };
    if *value == Json::Null {
        return Ok(Some(None));
    }
    let x = value.number("x")?;
    let y = value.number("y")?;
    if !(0.0..=1.0).contains(&x)
        || !(0.0..=1.0).contains(&y)
        || value.as_object().map_or(0, |fields| fields.len()) != 2
    {
        return Err(Error::bad("focus", "{x, y} with both from 0 to 1, or null"));
    }
    Ok(Some(Some([x, y])))
}

pub(super) fn item_anchor(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let pinned = a.flag("pinned", false)?;
    let mode = a
        .field("anchor")
        .map(anchor)
        .transpose()?
        .map_or(AnchorMode::Current(pinned), AnchorMode::Explicit);
    Ok(Op::ItemAnchor {
        item: a.target("item")?,
        mode,
    })
}

pub(super) fn item_lane(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let destination = if let Some(lane) = a.opt_target("lane")? {
        LaneMove::To(lane)
    } else {
        match a.opt_string("dir")?.as_deref() {
            Some("up") => LaneMove::Up,
            Some("down") => LaneMove::Down,
            _ => {
                return Err(Error::bad(
                    "lane or dir",
                    "a lane ID or a direction of up or down",
                ));
            }
        }
    };
    Ok(Op::ItemLane {
        items: a.targets("items")?,
        destination,
    })
}

pub(super) fn overlay_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::OverlaySet {
        item: a.target("item")?,
        placement: a
            .opt_string("placement")?
            .as_deref()
            .map(placement)
            .transpose()?,
        behind_presenter: a.opt_flag("behindPresenter")?,
        muted: a.opt_flag("muted")?,
    })
}

pub(super) fn lane_create(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::LaneCreate {
        kind: lane_kind(a.string("kind")?)?,
        name: a.opt_string("name")?,
    })
}

pub(super) fn lane_rename(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::LaneRename {
        lane: a.target("lane")?,
        name: a.string("name")?.into(),
    })
}

pub(super) fn lane_delete(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::LaneDelete {
        lane: a.target("lane")?,
    })
}

pub(super) fn component_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ComponentSet {
        clip: a.target("clip")?,
        component: component(a.string("component")?)?,
        enabled: a.flag("enabled", false)?,
    })
}

pub(super) fn audio_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let clip = match a.opt_target("clip")? {
        Some(clip) => {
            let component = component(a.string("component")?)?;
            if !matches!(component, Component::Mic | Component::System) {
                return Err(Error::bad("component", "mic or system"));
            }
            Some((clip, component))
        }
        None => None,
    };
    let items = match a.field("items") {
        Some(_) => a.targets("items")?,
        None => Vec::new(),
    };
    if clip.is_some() == !items.is_empty() {
        return Err(Error::bad(
            "clip or items",
            "a take clip with component, or sound items",
        ));
    }
    let bake = a
        .field("duck")
        .and_then(|duck| duck.field("bake"))
        .is_some_and(|bake| bake.as_bool() == Some(true));
    let duck = match a.field("duck") {
        None => None,
        Some(Json::Null) => Some(None),
        Some(duck) => {
            let amount = duck.number("amount")?;
            let attack = duck.opt_int("attack")?.unwrap_or(8);
            let release = duck.opt_int("release")?.unwrap_or(15);
            if !(0.0..=1.0).contains(&amount)
                || !(0..=120).contains(&attack)
                || !(0..=240).contains(&release)
            {
                return Err(Error::bad(
                    "duck",
                    "{amount 0–1, attack 0–120, release 0–240 frames}",
                ));
            }
            Some(Some(Duck {
                amount,
                attack,
                release,
            }))
        }
    };
    let normalize = a.opt_number("normalize")?;
    if normalize.is_some_and(|lufs| !(-40.0..=-5.0).contains(&lufs)) {
        return Err(Error::bad("normalize", "a LUFS target from -40 to -5"));
    }
    let volume = a.opt_within("volume", 0.0, 2.0)?;
    if clip.is_some() && volume.is_none() {
        return Err(Error::bad("volume", "a gain from 0 to 2"));
    }
    if !items.is_empty() && volume.is_none() && duck.is_none() && normalize.is_none() {
        return Err(Error::bad(
            "volume, duck or normalize",
            "at least one change for the items",
        ));
    }
    Ok(Op::AudioSet {
        clip,
        items,
        volume,
        duck,
        normalize,
        bake,
    })
}

pub(super) fn cursor_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CursorSet {
        clip: a.target("clip")?,
        size: a.opt_within("size", 0.5, 4.0)?,
        click_rings: a.opt_flag("clickRings")?,
    })
}

pub(super) fn keys_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::KeysSet {
        clip: a.target("clip")?,
        size: a.opt_within("size", 0.5, 3.0)?,
    })
}

pub(super) fn keys_hide(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let mut events = Vec::new();
    for value in list(a.required("events")?) {
        let index = value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| Error::bad("events", "nonnegative 32-bit keystroke event indices"))?;
        events.push(index);
    }
    Ok(Op::KeysHide {
        clip: a.target("clip")?,
        events,
        hidden: a.flag("hidden", false)?,
    })
}

pub(super) fn marker_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::MarkerAdd {
        at: a.frame("at")?,
        name: a.opt_string("name")?,
    })
}

pub(super) fn marker_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::MarkerSet {
        marker: a.target("marker")?,
        name: a.opt_string("name")?,
        at: a.opt_frame("at")?,
    })
}

pub(super) fn project_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ProjectSet {
        aspect: a.opt_string("aspect")?.as_deref().map(aspect).transpose()?,
        access: a.opt_string("access")?.as_deref().map(access).transpose()?,
    })
}

pub(super) fn clip_crop(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let rect = crop(a)?;
    let fit = a.opt_string("fit")?.as_deref().map(fit).transpose()?;
    let aspect = match a.opt_string("aspect")?.as_deref() {
        None => None,
        Some("canvas") => Some(None),
        Some(shape) => {
            let ratio = shape
                .split_once(':')
                .and_then(|(w, h)| Some(w.parse::<f64>().ok()? / h.parse::<f64>().ok()?))
                .filter(|ratio| ratio.is_finite() && *ratio > 0.05 && *ratio < 20.0)
                .ok_or_else(|| Error::bad("aspect", "9:16, 4:5, 1:1, 4:3, 16:9 or canvas"))?;
            Some(Some(ratio))
        }
    };
    let focus = focus(a)?.flatten();
    if rect.is_some() && aspect.is_some() {
        return Err(Error::bad("rect", "either rect or aspect, not both"));
    }
    if rect.is_none() && fit.is_none() && aspect.is_none() {
        return Err(Error::bad(
            "rect or fit",
            "a crop, a fit mode, an aspect, or a mix",
        ));
    }
    Ok(Op::ClipCrop {
        items: a.targets("items")?,
        rect,
        fit,
        aspect,
        focus,
    })
}

pub(super) fn cut_range(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CutRange {
        from: a.frame("from")?,
        to: a.frame("to")?,
        ripple: a.flag("ripple", true)?,
        connected: connected(a)?,
    })
}

pub(super) fn cut_words(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CutWords {
        words: list(a.required("words")?)
            .iter()
            .map(|value| word(value, "words"))
            .collect::<Result<Vec<_>, _>>()?,
        transcript_hash: hash(a)?,
        connected: connected(a)?,
    })
}

pub(super) fn effect_add(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::EffectAdd {
        target: a.target("target")?,
        kind: a.string("type")?.into(),
        params: effect_params(a, &mut scratch.params)?,
        enabled: a.opt_flag("enabled")?,
    })
}

pub(super) fn effect_set(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    let target = if let Some(effect) = a.opt_target("effect")? {
        EffectSelection::Id(effect)
    } else {
        EffectSelection::Owner {
            target: a.target("target")?,
            kind: a.string("type")?.into(),
        }
    };
    let mut reset = Vec::new();
    if let Some(words) = a.field("reset") {
        for word in list(words) {
            reset.push(
                word.as_str()
                    .ok_or_else(|| Error::bad("reset", "parameter names"))?
                    .to_owned(),
            );
        }
    }
    Ok(Op::EffectSet {
        target,
        params: effect_params(a, &mut scratch.params)?,
        enabled: a.opt_flag("enabled")?,
        reset,
    })
}

pub(super) fn effect_remove(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let effects = a
        .field("effects")
        .map(|value| targets(value, "effects"))
        .transpose()?
        .unwrap_or_default();
    Ok(Op::EffectRemove {
        effects,
        target: a.opt_target("target")?,
        kind: a.opt_string("type")?,
    })
}

pub(super) fn effect_paste(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let replace = match a.opt_string("mode")?.as_deref() {
        None | Some("merge") => false,
        Some("replace") => true,
        _ => return Err(Error::bad("mode", "merge or replace")),
    };
    Ok(Op::EffectPaste {
        from: a.target("from")?,
        to: a.targets("to")?,
        replace,
    })
}

pub(super) fn effect_promote(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let to_project = match a.string("to")? {
        "project" => true,
        "lane" => false,
        _ => return Err(Error::bad("to", "lane or project")),
    };
    Ok(Op::EffectPromote {
        effect: a.target("effect")?,
        to_project,
        clear_overrides: a.flag("clearOverrides", false)?,
    })
}

pub(super) fn captions_generate(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let max_words = a.opt_int("maxWords")?.unwrap_or(3);
    if !(1..=8).contains(&max_words) {
        return Err(Error::bad("maxWords", "an integer from 1 to 8"));
    }
    let style = a.opt_string("style")?;
    if let Some(style) = &style
        && !caption_presets()
            .iter()
            .any(|known| known.as_str() == Some(style))
    {
        return Err(Error::bad(
            "style",
            "a caption preset from commands.describe {id:\"caption.set\"}",
        ));
    }
    Ok(Op::CaptionsGenerate {
        clips: a
            .field("clips")
            .map(|values| targets(values, "clips"))
            .transpose()?,
        max_words,
        fillers: a.flag("fillers", false)?,
        lane: a.opt_target("lane")?,
        style,
    })
}

pub(super) fn caption_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CaptionSet {
        item: a.target("item")?,
        text: a.opt_string("text")?,
        words: caption_patches(a)?,
    })
}

pub(super) fn caption_split(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CaptionSplit {
        item: a.target("item")?,
        word: word(a.required("word")?, "word")?,
    })
}

pub(super) fn caption_merge(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let items = a.targets("items")?;
    if items.len() < 2 {
        return Err(Error::bad("items", "two or more caption IDs"));
    }
    Ok(Op::CaptionMerge { items })
}

pub(super) fn transcript_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let mut words = Vec::new();
    for word in a
        .field("words")
        .and_then(Json::as_array)
        .ok_or_else(|| Error::bad("words", "an array of {text, from, to}"))?
    {
        let text = word.string("text")?.trim();
        let bounds = || Error::bad("words", "integer source frames from and to");
        let from = word.opt_int("from")?.ok_or_else(bounds)?;
        let to = word.opt_int("to")?.ok_or_else(bounds)?;
        if text.is_empty()
            || from < 0
            || to <= from
            || words
                .last()
                .is_some_and(|w: &(String, Frame, Frame)| from < w.1)
        {
            return Err(Error::bad(
                "words",
                "nonempty text with increasing source frames, to after from",
            ));
        }
        words.push((text.to_owned(), from, to));
    }
    let language = a.opt_string("language")?.unwrap_or_else(|| "en-US".into());
    Ok(Op::TranscriptSet {
        media: a.target("media")?,
        words,
        language,
    })
}

// ── 0.3 rows (PLAN.md T16–T21) ──

fn frames_list(a: &Json, key: &str) -> Result<Vec<(Frame, Frame)>, Error> {
    let mut ranges = Vec::new();
    for range in a
        .field(key)
        .and_then(Json::as_array)
        .ok_or_else(|| Error::bad(key, "an array of [from, to] timeline frames"))?
    {
        let pair = range.as_array().filter(|pair| pair.len() == 2);
        let frames = pair.and_then(|pair| {
            Some((
                Frame::try_from(pair[0].as_i64()?).ok()?,
                Frame::try_from(pair[1].as_i64()?).ok()?,
            ))
        });
        match frames {
            Some((from, to)) if from >= 0 && to > from => ranges.push((from, to)),
            _ => return Err(Error::bad(key, "[from, to] pairs with to after from")),
        }
    }
    Ok(ranges)
}

fn text_fields(a: &Json, key: &str) -> Result<Vec<(String, String)>, Error> {
    let Some(fields) = a.field(key) else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for (name, value) in fields
        .as_object()
        .ok_or_else(|| Error::bad(key, "an object of text fields"))?
    {
        let text = value
            .as_str()
            .ok_or_else(|| Error::bad(&format!("{key}.{name}"), "text"))?;
        result.push((name.clone(), text.to_owned()));
    }
    Ok(result)
}

pub(super) fn clip_join(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ClipJoin {
        items: a.targets("items")?,
    })
}

pub(super) fn clip_ungroup(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ClipUngroup {
        item: a.target("item")?,
    })
}

pub(super) fn clip_restore(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ClipRestore {
        items: match a.field("items") {
            Some(_) => a.targets("items")?,
            None => Vec::new(),
        },
    })
}

pub(super) fn lane_reorder(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let to = a
        .field("to")
        .and_then(Json::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| Error::bad("to", "a stack index, 0 at the top"))?;
    Ok(Op::LaneReorder {
        lane: a.target("lane")?,
        to,
    })
}

pub(super) fn crop_keys(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let mut keys = Vec::new();
    for key in a
        .field("keys")
        .and_then(Json::as_array)
        .ok_or_else(|| {
            Error::bad(
                "keys",
                "an array of {at, x, y, w, h, ease}, or follow:\"cursor\" | \"face\" through splice mcp",
            )
        })?
    {
        let at = key
            .opt_int("at")?
            .ok_or_else(|| Error::bad("keys.at", "an owner frame"))?;
        let rect = [
            key.number("x")?,
            key.number("y")?,
            key.number("w")?,
            key.number("h")?,
        ];
        if rect[2] <= 0.0
            || rect[3] <= 0.0
            || rect[0] < 0.0
            || rect[1] < 0.0
            || rect[0] + rect[2] > 1.0 + 1e-9
            || rect[1] + rect[3] > 1.0 + 1e-9
            || keys
                .last()
                .is_some_and(|last: &(Frame, [f64; 4], Ease)| last.0 >= at)
        {
            return Err(Error::bad(
                "keys",
                "increasing frames with x, y, w, h inside 0…1 of the source",
            ));
        }
        keys.push((at, rect, ease(key, "ease")?));
    }
    if keys.is_empty() {
        return Err(Error::bad("keys", "at least one key"));
    }
    Ok(Op::CropKeys {
        item: a.target("item")?,
        keys,
    })
}

pub(super) fn transition_add(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::TransitionAdd {
        item: a.target("item")?,
        params: schema_params(a, "transition", &mut scratch.params)?,
    })
}

pub(super) fn transition_set(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::TransitionSet {
        item: a.target("item")?,
        params: schema_params(a, "transition", &mut scratch.params)?,
    })
}

pub(super) fn transition_remove(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::TransitionRemove {
        item: a.target("item")?,
    })
}

pub(super) fn mask_add(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::MaskAdd {
        item: a.target("item")?,
        params: schema_params(a, "mask", &mut scratch.params)?,
    })
}

pub(super) fn mask_set(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::MaskSet {
        mask: a.target("mask")?,
        params: schema_params(a, "mask", &mut scratch.params)?,
        enabled: a.opt_flag("enabled")?,
    })
}

pub(super) fn mask_track(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let mut keys = Vec::new();
    for key in a
        .field("keys")
        .and_then(Json::as_array)
        .ok_or_else(|| Error::bad("keys", "an array of {at, x, y} mask centers"))?
    {
        let at = key
            .opt_int("at")?
            .ok_or_else(|| Error::bad("keys.at", "an owner frame"))?;
        if keys
            .last()
            .is_some_and(|last: &(Frame, f64, f64)| last.0 >= at)
        {
            return Err(Error::bad("keys", "increasing frames"));
        }
        keys.push((at, key.number("x")?, key.number("y")?));
    }
    if keys.is_empty() {
        return Err(Error::bad(
            "keys",
            "at least one {at, x, y}; splice mcp fills them by tracking when omitted",
        ));
    }
    Ok(Op::MaskTrack {
        mask: a.target("mask")?,
        keys,
    })
}

pub(super) fn captions_translate(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let mut texts = Vec::new();
    for text in a.field("texts").map(list).unwrap_or(&[]) {
        texts.push(
            text.as_str()
                .ok_or_else(|| Error::bad("texts", "an array of strings, one per cue"))?
                .to_owned(),
        );
    }
    Ok(Op::CaptionsTranslate {
        lane: a.target("lane")?,
        language: a.string("language")?.into(),
        texts,
    })
}

pub(super) fn captions_import(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CaptionsImport {
        lane: a.opt_target("lane")?,
        text: a.string("text")?.into(),
    })
}

pub(super) fn audio_link(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::AudioLink {
        item: a.target("item")?,
    })
}

pub(super) fn audio_unlink(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::AudioUnlink {
        item: a.target("item")?,
        lane: a.opt_target("lane")?,
    })
}

pub(super) fn audio_sync(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::AudioSync {
        item: a.target("item")?,
        to: a.target("to")?,
        offset: a.opt_int("offset")?.ok_or_else(|| {
            Error::bad(
                "offset",
                "source frames; splice mcp measures it from the audio when omitted",
            )
        })?,
    })
}

pub(super) fn style_set(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    let params = schema_params(a, "brand", &mut scratch.params)?;
    if params.is_empty() {
        return Err(Error::bad(
            "font",
            "at least one of font, color, color2, accent, textColor, logo, logoPosition",
        ));
    }
    Ok(Op::StyleSet { params })
}

pub(super) fn cut_pauses(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CutPauses {
        min: a.opt_int("min")?,
        keep: a.opt_int("keep")?,
        basis: basis(a)?,
        activity: media_ranges(a)?,
    })
}

pub(super) fn intent_pace(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::IntentPace {
        max_pause: a.opt_int("maxPause")?,
        basis: basis(a)?,
        activity: media_ranges(a)?,
    })
}

pub(super) fn intent_fit_duration(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let seconds = a.number("seconds")?;
    if seconds.is_nan() || seconds <= 0.0 {
        return Err(Error::bad("seconds", "a positive duration"));
    }
    Ok(Op::IntentFitDuration {
        seconds,
        basis: basis(a)?,
        activity: media_ranges(a)?,
    })
}

fn basis(a: &Json) -> Result<Basis, Error> {
    match a.field("basis").map(|value| value.as_str()) {
        None => Ok(Basis::Auto),
        Some(Some("auto")) => Ok(Basis::Auto),
        Some(Some("transcript")) => Ok(Basis::Transcript),
        Some(Some("activity")) => Ok(Basis::Activity),
        Some(_) => Err(Error::bad("basis", "auto, transcript or activity")),
    }
}

/// `activity: [{media, from, to}]`: busy source frame ranges per take.
fn media_ranges(a: &Json) -> Result<Vec<(splice_model::MediaId, Frame, Frame)>, Error> {
    let expected = "[{media, from, to}] busy source frame ranges with to > from";
    let mut out = Vec::new();
    for range in a.field("activity").map(list).unwrap_or(&[]) {
        let media = range
            .field("media")
            .and_then(Json::as_str)
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| Error::bad("activity.media", "a med_… take ID"))?;
        match (range.opt_int("from")?, range.opt_int("to")?) {
            (Some(from), Some(to)) if to > from && from >= 0 => out.push((media, from, to)),
            _ => return Err(Error::bad("activity", expected)),
        }
    }
    Ok(out)
}

/// `[{from, to}]` source frame ranges under `key`.
fn source_ranges(a: &Json, key: &str) -> Result<Vec<(Frame, Frame)>, Error> {
    let mut out = Vec::new();
    for run in a.field(key).map(list).unwrap_or(&[]) {
        match (run.opt_int("from")?, run.opt_int("to")?) {
            (Some(from), Some(to)) if to > from => out.push((from, to)),
            _ => {
                return Err(Error::bad(
                    key,
                    "[{from, to}] source frame ranges with to > from",
                ));
            }
        }
    }
    Ok(out)
}

pub(super) fn intent_steps(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let activity = source_ranges(a, "activity")?;
    if activity.is_empty() {
        return Err(Error::bad(
            "activity",
            "[{from, to}] busy source frames; splice mcp fills them from the take's events when omitted",
        ));
    }
    let nonnegative = |key: &str| -> Result<Option<Frame>, Error> {
        match a.opt_int(key)? {
            Some(value) if value < 0 => Err(Error::rule(
                "BAD_ARGS",
                key,
                Some(Json::Signed(i64::from(value))),
                "frames ≥ 0",
                &format!("Pass {key} as a frame count of 0 or more."),
            )),
            value => Ok(value),
        }
    };
    Ok(Op::IntentSteps {
        item: a.target("item")?,
        lead: nonnegative("lead")?,
        settle: nonnegative("settle")?,
        max_gap: nonnegative("maxGap")?,
        retake_prefix: a.opt_string("retakePrefix")?,
        activity,
    })
}

pub(super) fn intent_speed_typing(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let typing = source_ranges(a, "typing")?;
    if typing.is_empty() {
        return Err(Error::bad(
            "typing",
            "[{from, to}] typing source frames; splice mcp fills them from the take's events when omitted",
        ));
    }
    let min_run = a.opt_int("minRun")?;
    if let Some(run) = min_run.filter(|&run| run < 1) {
        return Err(Error::rule(
            "BAD_ARGS",
            "minRun",
            Some(Json::Signed(i64::from(run))),
            "frames ≥ 1",
            "Pass minRun as the shortest typing run to speed up, in frames.",
        ));
    }
    Ok(Op::IntentSpeedTyping {
        item: a.target("item")?,
        factor: a.opt_within("factor", 1.25, 16.0)?.unwrap_or(3.0),
        min_run,
        typing,
    })
}

pub(super) fn caption_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let text = a.string("text")?.trim().to_owned();
    if text.is_empty() {
        return Err(Error::bad("text", "the cue's text"));
    }
    let from = a
        .opt_int("from")?
        .ok_or_else(|| Error::bad("from", "the cue's first timeline frame"))?;
    let to = a
        .opt_int("to")?
        .ok_or_else(|| Error::bad("to", "the timeline frame after the cue"))?;
    if from < 0 || to <= from {
        return Err(Error::rule(
            "BAD_ARGS",
            "to",
            Some(Json::Signed(i64::from(to))),
            "0 ≤ from < to",
            "Pass from and to as timeline frames with to after from.",
        ));
    }
    Ok(Op::CaptionAdd {
        text,
        from,
        to,
        lane: a.opt_target("lane")?,
    })
}

pub(super) fn intent_keep_shots(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let ranges = frames_list(a, "ranges")?;
    // An empty list would keep nothing and delete the whole timeline (M7).
    if ranges.is_empty() {
        return Err(Error::bad(
            "ranges",
            "at least one [from, to] range to keep",
        ));
    }
    Ok(Op::IntentKeepShots { ranges })
}

pub(super) fn intent_auto_zoom(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let mut clicks = Vec::new();
    for click in a.field("clicks").map(list).unwrap_or(&[]) {
        let at = click
            .opt_int("at")?
            .ok_or_else(|| Error::bad("clicks.at", "a source frame"))?;
        let (x, y) = (click.number("x")?, click.number("y")?);
        // Refused, not clamped: an off-screen click means the caller mixed up coordinates.
        for (name, value) in [("clicks.x", x), ("clicks.y", y)] {
            if !(0.0..=1.0).contains(&value) {
                return Err(Error::rule(
                    "BAD_ARGS",
                    name,
                    Some(Json::Float(value)),
                    "a position from 0 to 1 in the source frame",
                    "Pass normalized click positions, as takes.events reports them.",
                ));
            }
        }
        clicks.push((at, x, y));
    }
    if clicks.is_empty() {
        return Err(Error::bad(
            "clicks",
            "{at, x, y} source clicks; splice mcp fills them from the take's events when omitted",
        ));
    }
    Ok(Op::IntentAutoZoom {
        item: a.target("item")?,
        clicks,
        typing: source_ranges(a, "typing")?,
        amount: zoom_amount(a)?,
    })
}

pub(super) fn item_roll(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ItemRoll {
        item: a.target("item")?,
        by: a
            .opt_int("by")?
            .ok_or_else(|| Error::bad("by", "signed frames"))?,
    })
}

pub(super) fn item_slip(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ItemSlip {
        item: a.target("item")?,
        by: a
            .opt_int("by")?
            .ok_or_else(|| Error::bad("by", "signed source frames"))?,
    })
}

pub(super) fn item_slide(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ItemSlide {
        item: a.target("item")?,
        by: a
            .opt_int("by")?
            .ok_or_else(|| Error::bad("by", "signed frames"))?,
    })
}

pub(super) fn clip_freeze(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let length = a.opt_int("length")?.unwrap_or(60);
    if length < 3 {
        return Err(Error::bad("length", "at least 3 frames"));
    }
    Ok(Op::ClipFreeze {
        item: a.target("item")?,
        at: a.frame("at")?,
        length,
    })
}

pub(super) fn clip_compound(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ClipCompound {
        items: a.targets("items")?,
    })
}

pub(super) fn clip_uncompound(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ClipUncompound {
        item: a.target("item")?,
    })
}

fn param_path(a: &Json, reuse: &mut String) -> Result<String, Error> {
    let path = a.string("path")?;
    match path.split_once('.') {
        Some((kind, name)) if !kind.is_empty() && !name.is_empty() => Ok(reuse_string(reuse, path)),
        _ => Err(Error::bad(
            "path",
            "type.param, e.g. transform.scale or volume.gain",
        )),
    }
}

pub(super) fn param_set(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::ParamSet {
        target: a.target("target")?,
        path: param_path(a, &mut scratch.path)?,
        value: a.number("value")?,
    })
}

pub(super) fn param_keys(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let merge = match a.opt_string("mode")?.as_deref() {
        None | Some("replace") => false,
        Some("merge") => true,
        _ => return Err(Error::bad("mode", "replace or merge")),
    };
    Ok(Op::ParamKeys {
        target: a.target("target")?,
        path: param_path(a, &mut String::new())?,
        keys: keys(a, "keys")?,
        merge,
    })
}

fn normalized_rect(a: &Json, key: &str) -> Result<Option<Rect>, Error> {
    let Some(value) = a.field(key) else {
        return Ok(None);
    };
    let rect = Rect {
        x: value.number("x")?,
        y: value.number("y")?,
        w: value.number("w")?,
        h: value.number("h")?,
    };
    if ![rect.x, rect.y, rect.w, rect.h]
        .iter()
        .all(|n| n.is_finite())
        || rect.w.abs() > 4.0
        || rect.h.abs() > 4.0
    {
        return Err(Error::bad(key, "{x, y, w, h} normalized to the canvas"));
    }
    Ok(Some(rect))
}

pub(super) fn generator_add(a: &Json, scratch: &mut DecodeScratch) -> DecodedOp {
    let shape = ShapeKind::parse(a.string("shape")?).ok_or_else(|| {
        Error::bad(
            "shape",
            "solid, gradient, rect, ellipse, arrow, line or callout",
        )
    })?;
    Ok(Op::GeneratorAdd {
        shape,
        at: a.frame("at")?,
        length: a.opt_int("length")?,
        rect: normalized_rect(a, "rect")?,
        lane: a.opt_target("lane")?,
        params: schema_params(a, "shape", &mut scratch.params)?,
    })
}

pub(super) fn template_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let name = a.string("name")?;
    if ![
        "lower-third",
        "callout",
        "cta",
        "title-card",
        "chapter-card",
    ]
    .contains(&name)
    {
        return Err(Error::bad(
            "name",
            "lower-third, callout, cta, title-card or chapter-card",
        ));
    }
    Ok(Op::TemplateAdd {
        name: name.into(),
        at: a.frame("at")?,
        length: a.opt_int("length")?,
        fields: text_fields(a, "fields")?,
    })
}

pub(super) fn template_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::TemplateSet {
        item: a.target("item")?,
        fields: text_fields(a, "fields")?,
    })
}

pub(super) fn take_pointer(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let expected = "[{at, x, y, action}] with x, y in 0–1 and action move or click";
    let mut marks = Vec::new();
    for mark in list(a.required("marks")?) {
        let unit = |key: &str| {
            mark.field(key)
                .and_then(Json::as_f64)
                .filter(|v| (0.0..=1.0).contains(v))
                .ok_or_else(|| Error::bad("marks", expected))
        };
        let at = mark
            .field("at")
            .and_then(Json::as_i64)
            .and_then(|at| Frame::try_from(at).ok())
            .ok_or_else(|| Error::bad("marks", expected))?;
        let click = match mark.field("action").and_then(Json::as_str) {
            None | Some("move") => false,
            Some("click") => true,
            Some(_) => return Err(Error::bad("marks", expected)),
        };
        marks.push((at, unit("x")?, unit("y")?, click));
    }
    if marks.is_empty() {
        return Err(Error::bad("marks", expected));
    }
    Ok(Op::TakePointer {
        clip: a.target("clip")?,
        marks,
    })
}

pub(super) fn media_relink(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let path = a.string("path")?;
    if path.is_empty() || path.contains('\0') {
        return Err(Error::bad("path", "a file path"));
    }
    Ok(Op::MediaRelink {
        media: a.target("media")?,
        path: path.into(),
        frames: a.opt_int("frames")?,
        force: a.flag("force", false)?,
    })
}

pub(super) fn media_consolidate(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::MediaConsolidate {
        media: match a.field("media") {
            Some(_) => a.targets("media")?,
            None => Vec::new(),
        },
    })
}
