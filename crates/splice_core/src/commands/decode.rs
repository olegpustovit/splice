use super::{CommandError as Error, DecodeScratch, Json, args::*, op::*};
use splice_model::{Duck, Fit, Placement};

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
        "motion" => OverlayContent::Motion {
            from: a.opt_number("from")?.unwrap_or(1.0),
            to: a.opt_number("to")?.unwrap_or(1.32),
            focus: focus(a)?.flatten(),
            ease: a.flag("ease", false)?,
        },
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
        _ => return Err(Error::bad("kind", "title, motion, or media")),
    };
    Ok(Op::OverlayAdd {
        content,
        at: a.opt_frame("at")?,
        anchor: a.field("anchor").map(anchor).transpose()?,
        lane: a.opt_target("lane")?,
        length: a.opt_int("length")?,
    })
}

pub(super) fn title_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::TitleAdd {
        at: a.frame("at")?,
        length: a.opt_int("length")?.unwrap_or(90),
        text: a.opt_string("text")?,
        lane: a.opt_target("lane")?,
    })
}

pub(super) fn title_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::TitleSet {
        item: a.target("item")?,
        text: a.string("text")?.into(),
    })
}

pub(super) fn motion_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::MotionAdd {
        at: a.frame("at")?,
        length: a.opt_int("length")?.unwrap_or(120),
        from: a.opt_number("from")?.unwrap_or(1.0),
        to: a.opt_number("to")?.unwrap_or(1.32),
        focus: focus(a)?.flatten(),
        ease: a.flag("ease", false)?,
    })
}

pub(super) fn motion_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::MotionSet {
        item: a.target("item")?,
        from: a.opt_number("from")?,
        to: a.opt_number("to")?,
        focus: focus(a)?,
        ease: a.opt_flag("ease")?,
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
    let volume = a.opt_number("volume")?;
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
    })
}

pub(super) fn cursor_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CursorSet {
        clip: a.target("clip")?,
        size: a.opt_number("size")?,
        click_rings: a.opt_flag("clickRings")?,
    })
}

pub(super) fn keys_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::KeysSet {
        clip: a.target("clip")?,
        size: a.opt_number("size")?,
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
    })
}

pub(super) fn cut_words(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::CutWords {
        words: list(a.required("words")?)
            .iter()
            .map(|value| word(value, "words"))
            .collect::<Result<Vec<_>, _>>()?,
        transcript_hash: hash(a)?,
    })
}

pub(super) fn effect_add(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    Ok(Op::EffectAdd {
        target: a.target("target")?,
        kind: a.string("type")?.into(),
        params: effect_params(a)?,
        enabled: a.opt_flag("enabled")?,
    })
}

pub(super) fn effect_set(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
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
        params: effect_params(a)?,
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
        && !CAPTION_PRESETS.contains(&style.as_str())
    {
        return Err(Error::bad("style", &CAPTION_PRESETS.join(", ")));
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
