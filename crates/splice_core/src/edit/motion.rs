//! Text and motion (PLAN.md T42, D16, D20): spring settle reports, inline title looks that
//! compile to `layout`/`text`/`animate` effects and transform keys, motion presets as ordinary
//! keys, and paint order.
use super::settle::{Owner, effects};
use super::time::item_slot;
use super::*;
use crate::commands::{Motion, MotionPreset, Order, TitleLook};

/// The slowest spring among `keys` and the first spring the next key cuts short.
#[derive(Clone, Copy, Default)]
pub(super) struct Springs {
    settle: Option<f64>,
    /// (key frame, settle frames, frames to the next key).
    cut: Option<(Frame, f64, Frame)>,
}

/// Measures the spring segments of sorted keys (the last key's ease never plays).
pub(super) fn springs(keys: &[Key]) -> Springs {
    let mut out = Springs::default();
    for pair in keys.windows(2) {
        let Some(settle) = pair[0].ease.settle() else {
            continue;
        };
        out.settle = Some(out.settle.map_or(settle, |s: f64| s.max(settle)));
        let gap = pair[1].at - pair[0].at;
        if out.cut.is_none() && f64::from(gap) < settle {
            out.cut = Some((pair[0].at, settle, gap));
        }
    }
    out
}

/// Transform parameters a motion writes, with their identity values and schema ranges.
const CHANNELS: [(&str, f64, [f64; 2]); 5] = [
    ("x", 0.0, [-2.0, 2.0]),
    ("y", 0.0, [-2.0, 2.0]),
    ("scale", 1.0, [0.05, 10.0]),
    ("rotation", 0.0, [-720.0, 720.0]),
    ("opacity", 1.0, [0.0, 1.0]),
];
const X: usize = 0;
const Y: usize = 1;
const SCALE: usize = 2;
const ROTATION: usize = 3;
const OPACITY: usize = 4;

/// Keys per transform channel, relative to the owner's start.
type Channels = [Vec<Key>; 5];

fn key(at: Frame, value: f64, ease: Ease) -> Key {
    Key { at, value, ease }
}

/// Default length of a one-shot preset, and the period of a repeating one.
fn default_frames(preset: MotionPreset) -> Frame {
    match preset {
        MotionPreset::BounceIn => 24,
        MotionPreset::Pop => 14,
        MotionPreset::Shake => 18,
        MotionPreset::Pulse => 30,
        MotionPreset::Float => 60,
        MotionPreset::Wiggle => 20,
        MotionPreset::Fade | MotionPreset::Sink | MotionPreset::Shrink | MotionPreset::Slide => 8,
    }
}

fn repeats(preset: MotionPreset) -> bool {
    matches!(
        preset,
        MotionPreset::Pulse | MotionPreset::Float | MotionPreset::Wiggle
    )
}

/// A decaying side-to-side shake over `[from, from + length]`, every two frames.
fn shake(out: &mut Vec<Key>, base: f64, from: Frame, length: Frame, amount: f64) {
    let steps = (length / 2).max(1);
    for step in 0..steps {
        let decay = 1.0 - f64::from(step) / f64::from(steps);
        let sign = if step % 2 == 0 { 1.0 } else { -1.0 };
        let value = if step == 0 {
            base
        } else {
            base + sign * amount * decay
        };
        out.push(key(from + step * 2, value, Ease::Linear));
    }
    out.push(key(from + length, base, Ease::Linear));
}

/// Back and forth between `base` and `base + amount` every half period over the window, back at
/// `base` when it ends.
fn oscillate(out: &mut Vec<Key>, base: f64, [from, to]: [Frame; 2], period: Frame, amount: f64) {
    let half = (period / 2).max(1);
    let mut at = from;
    let mut up = false;
    while at < to {
        out.push(key(at, if up { base + amount } else { base }, Ease::InOut));
        at += half;
        up = !up;
    }
    out.push(key(to, base, Ease::InOut));
}

/// The keys `motion` writes over `[from, to]` on top of the owner's `base` channel values.
/// One-shot presets use `motion.length` frames from `from`; repeating ones fill the window with
/// `period`-frame cycles.
fn compile(motion: Motion, [from, to]: [Frame; 2], period: Frame, base: [f64; 5]) -> Channels {
    let mut out: Channels = Default::default();
    let i = motion.intensity;
    let end = to.max(from + 1);
    match motion.preset {
        MotionPreset::BounceIn => {
            // A spring that settles inside the window: settle time scales with duration.
            let per_frame = Ease::Spring {
                duration: 30.0,
                bounce: 0.5,
            }
            .settle()
            .unwrap_or(30.0)
                / 30.0;
            let duration =
                ((f64::from(end - from) / per_frame * 100.0).floor() / 100.0).clamp(2.0, 600.0);
            let spring = Ease::Spring {
                duration,
                bounce: 0.5,
            };
            out[SCALE].push(key(from, base[SCALE] * (1.0 - 0.8 * i).max(0.05), spring));
            out[SCALE].push(key(end, base[SCALE], Ease::InOut));
            out[OPACITY].push(key(from, 0.0, Ease::Out));
            out[OPACITY].push(key(
                from + ((end - from) / 4).max(1),
                base[OPACITY],
                Ease::InOut,
            ));
        }
        MotionPreset::Pop => {
            let mid = from + ((end - from) / 2).max(1);
            out[SCALE].push(key(from, base[SCALE], Ease::Out));
            out[SCALE].push(key(mid, base[SCALE] * (1.0 + 0.3 * i), Ease::InOut));
            out[SCALE].push(key(end.max(mid + 1), base[SCALE], Ease::InOut));
        }
        MotionPreset::Shake => shake(&mut out[X], base[X], from, end - from, 0.03 * i),
        MotionPreset::Pulse => oscillate(
            &mut out[SCALE],
            base[SCALE],
            [from, end],
            period,
            base[SCALE] * 0.1 * i,
        ),
        MotionPreset::Float => {
            oscillate(&mut out[Y], base[Y], [from, end], period, -0.03 * i);
        }
        MotionPreset::Wiggle => {
            oscillate(
                &mut out[ROTATION],
                base[ROTATION],
                [from, end],
                period,
                8.0 * i,
            );
            // Swing both ways: the second half of each cycle mirrors the first.
            for (index, k) in out[ROTATION].iter_mut().enumerate() {
                if index % 4 == 3 {
                    k.value = base[ROTATION] - 8.0 * i;
                }
            }
        }
        MotionPreset::Fade | MotionPreset::Sink | MotionPreset::Shrink | MotionPreset::Slide => {
            out[OPACITY].push(key(from, base[OPACITY], Ease::In));
            out[OPACITY].push(key(end, 0.0, Ease::In));
            let (channel, target) = match motion.preset {
                MotionPreset::Sink => (Y, base[Y] + 0.08 + 0.12 * i),
                MotionPreset::Shrink => (SCALE, base[SCALE] * (0.6 - 0.5 * i)),
                MotionPreset::Slide => (X, base[X] - 0.15 - 0.3 * i),
                _ => return out,
            };
            out[channel].push(key(from, base[channel], Ease::In));
            out[channel].push(key(end, target, Ease::In));
        }
    }
    for (channel, (_, _, [low, high])) in out.iter_mut().zip(CHANNELS) {
        for k in channel.iter_mut() {
            k.value = k.value.clamp(low, high);
        }
    }
    out
}

/// A parameter of `kind` resolved through `scopes` (project, lane, item: the last one set wins).
fn resolved(scopes: &[&[Effect]], kind: &str, name: &str) -> Option<EffectValue> {
    let mut found = None;
    for scope in scopes {
        for effect in scope
            .iter()
            .filter(|e| e.kind == kind && e.enabled != Some(false))
        {
            if let Some(param) = effect.params.iter().find(|p| p.name == name) {
                found = Some(param.value.clone());
            }
        }
    }
    found
}

fn number_at(value: Option<&EffectValue>, at: f64, default: f64) -> f64 {
    match value {
        Some(EffectValue::Number(n)) => *n,
        Some(EffectValue::Keys(keys)) => splice_model::keys::sample(keys, at),
        _ => default,
    }
}

/// Union of two `[x, y, w, h]` boxes.
fn union(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    let Some(a) = a else { return b };
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    [
        x,
        y,
        (a[0] + a[2]).max(b[0] + b[2]) - x,
        (a[1] + a[3]).max(b[1] + b[3]) - y,
    ]
}

impl Editor {
    /// Reports spring settle times on the current change; a spring the next key interrupts is
    /// named in the summary so the agent can move the key.
    pub(super) fn report_springs(&mut self, springs: Springs) {
        let Some(settle) = springs.settle else {
            return;
        };
        let detail = self.detail.get_or_insert_with(ChangeDetail::default);
        detail.settle = Some(detail.settle.map_or(settle, |s| s.max(settle)));
        if let Some((at, settle, gap)) = springs.cut {
            write!(
                self.note,
                "; the spring at frame {at} needs {} frames to settle but the next key is {gap} \
                 frames later (it jumps there)",
                settle.ceil()
            )
            .expect("writing to a String");
        }
    }

    pub(super) fn run_motion(&mut self, op: Op) -> Result<Outcome, CommandError> {
        match op {
            Op::ItemOrder { item, to } => self.order(&item, &to),
            Op::MotionPreset {
                items,
                motion,
                at,
                sized,
            } => self.preset(&items, motion, at, sized),
            _ => unreachable!("motion dispatch"),
        }
    }

    /// `title.add` with an inline look: compiles it onto the new title and reports its bounds.
    pub(super) fn title_look(
        &mut self,
        outcome: Outcome,
        look: Option<&TitleLook>,
    ) -> Result<Outcome, CommandError> {
        let target = outcome.primary.clone().expect("created title");
        if let Some(look) = look {
            self.apply_look(&target, look)?;
        }
        self.report_bounds(&target)?;
        Ok(outcome)
    }

    /// `title.set`: new text and/or look, merged into what the title has.
    pub(super) fn edit_title(
        &mut self,
        target: &Target,
        text: Option<String>,
        look: Option<&TitleLook>,
    ) -> Result<Outcome, CommandError> {
        let target = self.resolve(target)?;
        let TimelineId::Item(id) = self.timeline(&target)? else {
            return Err(wrong_kind("Use a title item for title.set."));
        };
        let slot = item_slot(&self.project, id)?;
        if !matches!(
            self.project.items.data(slot).expect("live item").kind,
            ItemKind::Title { .. }
        ) {
            return Err(wrong_kind("Use a title item for title.set."));
        }
        let outcome = match text {
            Some(text) => self.set_title(&target, text)?,
            None => Outcome::edit(format!("Restyled title {id}"), target.clone()),
        };
        if let Some(look) = look {
            self.apply_look(&target, look)?;
        }
        self.report_bounds(&target)?;
        Ok(outcome)
    }

    fn apply_look(&mut self, target: &Target, look: &TitleLook) -> Result<(), CommandError> {
        let owner = self.owner(target)?;
        let mut created = Vec::new();
        if let Some(params) = &look.text_box {
            self.upsert(owner, "layout", params.clone(), None, &mut created)?;
            let scope: [&[Effect]; 1] = [effects(&self.project, owner)];
            let number =
                |name, default| number_at(resolved(&scope, "layout", name).as_ref(), 0.0, default);
            let (x, y, w, h) = (
                number("x", 0.06),
                number("y", 0.07),
                number("w", 0.88),
                number("h", 0.12),
            );
            if x + w > 1.0 + 1e-9 || y + h > 1.0 + 1e-9 {
                return Err(CommandError::rule(
                    "BAD_ARGS",
                    "box",
                    None,
                    &format!(
                        "a box inside the canvas: x + w ≤ 1 and y + h ≤ 1 (now x {x} + w {w}, y {y} + h {h})"
                    ),
                    "Shrink w or h, or move x or y, so the box ends inside the canvas; animate it \
                     off canvas with out, loop or param.keys transform instead.",
                ));
            }
        }
        if !look.style.is_empty() {
            self.upsert(owner, "text", look.style.clone(), None, &mut created)?;
        }
        if !look.enter.is_empty() {
            self.upsert(owner, "animate", look.enter.clone(), None, &mut created)?;
        }
        let Owner::Item(slot) = owner else {
            unreachable!("titles are items")
        };
        let length = self.project.items.timing(slot).expect("live item").length;
        let exit = match look.exit {
            Some(exit) if exit.length >= length => {
                return Err(CommandError::rule(
                    "BAD_ARGS",
                    "out.duration",
                    Some(Json::Signed(i64::from(exit.length))),
                    &format!("fewer frames than the title's length ({length})"),
                    "Pass a shorter out.duration, or lengthen the title first.",
                ));
            }
            Some(exit) => exit.length,
            None => 0,
        };
        if let Some(repeat) = look.repeat {
            self.write_motion(target, repeat, [0, length - exit], repeat.length)?;
        }
        if let Some(motion) = look.exit {
            self.write_motion(target, motion, [length - exit, length], exit)?;
        }
        Ok(())
    }

    /// Compiles `motion` over `window` into the owner's transform keys: keys inside the window
    /// are replaced, keys outside it stay, and constant values are the base it moves from.
    fn write_motion(
        &mut self,
        target: &Target,
        motion: Motion,
        window: [Frame; 2],
        period: Frame,
    ) -> Result<(), CommandError> {
        let owner = self.owner(target)?;
        let transform: Vec<EffectParam> = effects(&self.project, owner)
            .iter()
            .find(|effect| effect.kind == "transform" && effect.enabled != Some(false))
            .map(|effect| effect.params.clone())
            .unwrap_or_default();
        let value = |name: &str| transform.iter().find(|p| p.name == name);
        let mut base = [0.0; 5];
        for (slot, (name, identity, _)) in base.iter_mut().zip(CHANNELS) {
            *slot = number_at(
                value(name).map(|p| &p.value),
                f64::from(window[0]),
                identity,
            );
        }
        let compiled = compile(motion, window, period, base);
        let mut springs_seen = Springs::default();
        for ((name, _, _), new) in CHANNELS.into_iter().zip(compiled) {
            if new.is_empty() {
                continue;
            }
            let [from, to] = [new[0].at, new[new.len() - 1].at];
            let mut keys: Vec<Key> = match value(name).map(|p| &p.value) {
                Some(EffectValue::Keys(old)) => old
                    .iter()
                    .copied()
                    .filter(|k| k.at < from || k.at > to)
                    .collect(),
                _ => Vec::new(),
            };
            let measured = springs(&new);
            if measured.settle.is_some() {
                springs_seen = measured;
            }
            keys.extend(new);
            keys.sort_by_key(|k| k.at);
            self.set_param(
                target,
                &format!("transform.{name}"),
                EffectValue::Keys(keys),
                false,
            )?;
        }
        // set_param wrote its own summary; keep only what the springs add.
        self.note.clear();
        self.report_springs(springs_seen);
        Ok(())
    }

    /// `motion.preset`: the preset's keys on every item, from `at` for `length` frames (one-shot
    /// presets default to their own length, repeating ones to the rest of the item).
    fn preset(
        &mut self,
        items: &[Target],
        motion: Motion,
        at: Frame,
        sized: bool,
    ) -> Result<Outcome, CommandError> {
        let mut count = 0;
        for target in items {
            let id = self.timeline(target)?;
            let length = match id {
                TimelineId::Clip(_) | TimelineId::Item(_) => {
                    let span = time::span(&self.project, id)?;
                    span.end - span.start
                }
                TimelineId::Marker(_) => {
                    return Err(wrong_kind("Pass clips or items (clp_… or ovl_…)."));
                }
            };
            if let TimelineId::Item(item) = id {
                let slot = item_slot(&self.project, item)?;
                if matches!(
                    self.project.items.data(slot).expect("live item").kind,
                    ItemKind::Sound(_) | ItemKind::Zoom(_) | ItemKind::Motion(_)
                ) {
                    return Err(wrong_kind(
                        "motion.preset moves pictures, titles, shapes and captions; zooms are \
                         camera segments (zoom.set).",
                    ));
                }
            }
            let frames = if sized {
                motion.length
            } else if repeats(motion.preset) {
                length - at
            } else {
                default_frames(motion.preset)
            };
            if at + frames > length {
                return Err(CommandError::rule(
                    "BAD_ARGS",
                    if sized { "length" } else { "at" },
                    None,
                    &format!("at + length within the item ({length} frames)"),
                    &format!("Pass at + length ≤ {length} for {id}, or lengthen the item first."),
                ));
            }
            if frames < 2 {
                return Err(CommandError::rule(
                    "BAD_ARGS",
                    "at",
                    None,
                    &format!("at least 2 frames before the item ends ({length} frames)"),
                    "Pass an earlier at.",
                ));
            }
            let target = Target::Timeline(id);
            self.write_motion(
                &target,
                Motion {
                    length: frames,
                    ..motion
                },
                [at, at + frames],
                default_frames(motion.preset),
            )?;
            count += 1;
        }
        let name = MotionPreset::MOTION
            .iter()
            .find(|(preset, _)| *preset == motion.preset)
            .map_or("motion", |(_, name)| name);
        let summary = std::mem::take(&mut self.note);
        self.say(format_args!("Added {name} to {count} item(s){summary}"));
        let primary = items.first().cloned().expect("decoded items");
        Ok(Outcome::noted(self.resolve(&primary)?))
    }

    /// `item.order`: sets the item's z so it paints where asked; reports the z it got.
    fn order(&mut self, target: &Target, to: &Order) -> Result<Outcome, CommandError> {
        let id = self.visual_item(target)?;
        let slot = item_slot(&self.project, id)?;
        let others = || {
            self.project
                .items
                .order()
                .iter()
                .filter(|&&other| other != slot)
                .filter(|&&other| {
                    !matches!(
                        self.project.items.data(other).map(|d| &d.kind),
                        Some(ItemKind::Sound(_) | ItemKind::Zoom(_) | ItemKind::Motion(_))
                    )
                })
                .filter_map(|&other| self.project.items.data(other))
                .map(|data| data.appearance.z)
        };
        let z = match to {
            Order::Front => others().max().map_or(0, |z| z.saturating_add(1)),
            Order::Back => others().min().map_or(0, |z| z.saturating_sub(1)),
            Order::Above(other) | Order::Below(other) => {
                let other = self.visual_item(other)?;
                if other == id {
                    return Err(CommandError::bad("to", "another item than the one ordered"));
                }
                let z = self
                    .project
                    .items
                    .data(item_slot(&self.project, other)?)
                    .expect("live item")
                    .appearance
                    .z;
                if matches!(to, Order::Above(_)) {
                    z.saturating_add(1)
                } else {
                    z.saturating_sub(1)
                }
            }
            Order::Z(z) => *z,
        };
        self.project
            .items
            .data_mut(slot)
            .expect("live item")
            .appearance
            .z = z;
        self.detail.get_or_insert_with(ChangeDetail::default).z = Some(z);
        self.say(format_args!("Ordered {id} at z {z}"));
        Ok(Outcome::noted(Target::Timeline(TimelineId::Item(id))))
    }

    /// An item that paints: a media overlay, title, shape or caption.
    fn visual_item(&self, target: &Target) -> Result<ItemId, CommandError> {
        let TimelineId::Item(id) = self.timeline(target)? else {
            return Err(wrong_kind(
                "Pass an overlay, title, shape or caption (ovl_…); spine clips keep spine order.",
            ));
        };
        let slot = item_slot(&self.project, id)?;
        if matches!(
            self.project.items.data(slot).expect("live item").kind,
            ItemKind::Sound(_) | ItemKind::Zoom(_) | ItemKind::Motion(_)
        ) {
            return Err(wrong_kind(
                "Sounds and zooms do not paint; pass an overlay, title, shape or caption.",
            ));
        }
        Ok(id)
    }

    /// Reports a title's text box at rest (its last fully opaque frame, before any fade-out)
    /// and swept over every frame by its transform keys and unit animator.
    fn report_bounds(&mut self, target: &Target) -> Result<(), CommandError> {
        let TimelineId::Item(id) = self.timeline(target)? else {
            return Ok(());
        };
        let slot = item_slot(&self.project, id)?;
        let project = &self.project;
        let data = project.items.data(slot).expect("live item");
        let lane = project
            .lanes
            .iter()
            .find(|lane| lane.id == data.lane)
            .map_or(&[][..], |lane| &lane.effects[..]);
        let scopes: [&[Effect]; 3] = [&project.effects, lane, &data.appearance.effects];
        let [cw, ch] = project.size.map(f64::from);
        let number =
            |kind, name, default| number_at(resolved(&scopes, kind, name).as_ref(), 0.0, default);
        let has_box = scopes.iter().any(|scope| {
            scope
                .iter()
                .any(|e| e.kind == "layout" && e.enabled != Some(false))
        });
        // Normalized box: the layout effect's, or the default band the evaluator draws.
        let rest = if has_box {
            [
                number("layout", "x", 0.06),
                number("layout", "y", 0.07),
                number("layout", "w", 0.88),
                number("layout", "h", 0.12),
            ]
        } else {
            let size = number("text", "size", 0.06);
            let line = number("text", "lineHeight", 1.15);
            let h = size * cw.min(ch) / 0.6 * line / 1.15 / ch;
            let y = match resolved(&scopes, "text", "position") {
                Some(EffectValue::Text(p)) if p == "middle" => (1.0 - h) / 2.0,
                Some(EffectValue::Text(p)) if p == "bottom" => 0.86 - h,
                Some(EffectValue::Text(p)) if p == "lowerThird" => 0.76,
                _ => 0.07,
            };
            [0.06, y, 0.88, h]
        };
        // Unit animators move units inside the box (gpu text reveal): rise from 0.35 h below,
        // slide from 0.08 w left, bounce up to ~1.1×.
        let mut grown = rest;
        match resolved(&scopes, "animate", "preset") {
            Some(EffectValue::Text(p)) if p == "rise" => grown[3] *= 1.35,
            Some(EffectValue::Text(p)) if p == "slide" => {
                grown[0] -= rest[2] * 0.08;
                grown[2] *= 1.08;
            }
            Some(EffectValue::Text(p)) if p == "bounce" => {
                grown = [
                    rest[0] - rest[2] * 0.05,
                    rest[1] - rest[3] * 0.05,
                    rest[2] * 1.1,
                    rest[3] * 1.1,
                ];
            }
            _ => {}
        }
        let transform: Vec<(String, Option<EffectValue>)> = [
            "x", "y", "scale", "rotation", "anchorX", "anchorY", "opacity",
        ]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                resolved(&[&data.appearance.effects], "transform", name),
            )
        })
        .collect();
        let length = project.items.timing(slot).expect("live item").length;
        let place = |frame: Frame, [bx, by, bw, bh]: [f64; 4]| {
            let at = f64::from(frame);
            let get = |index: usize, default| number_at(transform[index].1.as_ref(), at, default);
            let (dx, dy, scale, rotation) =
                (get(0, 0.0) * cw, get(1, 0.0) * ch, get(2, 1.0), get(3, 0.0));
            let (x, y, w, h) = (bx * cw, by * ch, bw * cw, bh * ch);
            let pivot = [x + get(4, 0.5) * w, y + get(5, 0.5) * h];
            let (sin, cos) = rotation.to_radians().sin_cos();
            let mut bounds: Option<[f64; 4]> = None;
            for [px, py] in [[x, y], [x + w, y], [x, y + h], [x + w, y + h]] {
                let (ox, oy) = ((px - pivot[0]) * scale, (py - pivot[1]) * scale);
                let corner = [
                    pivot[0] + ox * cos - oy * sin + dx,
                    pivot[1] + ox * sin + oy * cos + dy,
                ];
                bounds = Some(union(bounds, [corner[0], corner[1], 0.0, 0.0]));
            }
            let [x, y, w, h] = bounds.expect("four corners");
            [x / cw, y / ch, w / cw, h / ch]
        };
        let opacity = |frame: Frame| number_at(transform[6].1.as_ref(), f64::from(frame), 1.0);
        let peak = (0..length).map(opacity).fold(0.0, f64::max);
        let last = (0..length)
            .rev()
            .find(|&frame| opacity(frame) >= peak - 1e-9)
            .unwrap_or(0);
        let end = place(last, rest);
        let mut swept = None;
        for frame in 0..length {
            swept = Some(union(swept, place(frame, grown)));
            swept = Some(union(swept, place(frame, rest)));
        }
        let detail = self.detail.get_or_insert_with(ChangeDetail::default);
        detail.bounds = Some([end, swept.unwrap_or(end)]);
        Ok(())
    }
}
