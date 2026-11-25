use super::{surface::Surface, viewer_captions::Guide, viewer_error};
use pxui::{
    Fill, Flags, Handle, Image, ImageStyle, KeyCode, Node, Point, Rect, Rgba, Signal, Ui,
    draw::Fit, ui::Size,
};
use splice_av::{Composition, Player};
use splice_core::history::{Store, StoreError};
use splice_kit::Phase;
use std::rc::Rc;

pub(super) struct Program {
    composition: Rc<Composition>,
    player: Player,
    shown: Option<i32>,
    pending: Option<i32>,
    rate: i8,
    scrub_origin: Option<i32>,
}

impl Program {
    pub(super) fn prepare(store: &Store) -> Result<Option<Self>, StoreError> {
        let Some(composition) = composition(store)? else {
            return Ok(None);
        };
        let player = composition.player().map_err(|error| viewer_error(&error))?;
        Ok(Some(Self {
            composition,
            player,
            shown: None,
            pending: None,
            rate: 0,
            scrub_origin: None,
        }))
    }

    pub(super) fn size(&self) -> [u32; 2] {
        self.composition.size()
    }

    pub(super) fn evaluator(&self) -> &splice_av::Evaluator {
        self.composition.evaluator()
    }

    pub(super) fn sample(&mut self, surface: &mut Surface) -> Result<(), StoreError> {
        if let Some(at) = surface.sample(&self.composition, &mut self.player)? {
            self.shown = Some(at);
            if self.pending == Some(at) || self.playing() {
                self.pending = None;
            }
        }
        if self.playing() && self.player.ended() {
            let boundary = if self.rate < 0 { 0 } else { self.end() - 1 };
            self.set_rate(0);
            if self.shown != Some(boundary) {
                self.seek(boundary)?;
            }
        }
        Ok(())
    }

    pub(super) fn at(&self) -> i32 {
        self.pending.or(self.shown).unwrap_or(0)
    }

    pub(super) fn end(&self) -> i32 {
        self.composition.end()
    }

    pub(super) fn playing(&self) -> bool {
        self.rate != 0
    }

    pub(super) fn rate(&self) -> i8 {
        self.rate
    }

    pub(super) fn dragging(&self) -> bool {
        self.scrub_origin.is_some()
    }

    pub(super) fn wants_frame(&self) -> bool {
        self.playing() || self.pending.is_some() || self.shown.is_none()
    }

    pub(super) fn set_rate(&mut self, rate: i8) {
        let rate = rate.clamp(-8, 8);
        if rate != self.rate {
            self.player.set_rate(rate);
            self.rate = rate;
        }
    }

    pub(super) fn toggle(&mut self) -> Result<(), StoreError> {
        let rate = i8::from(!self.playing());
        self.start(rate)
    }

    pub(super) fn shuttle(&mut self, direction: i8) -> Result<(), StoreError> {
        self.start(shuttle_rate(self.rate, direction))
    }

    fn start(&mut self, rate: i8) -> Result<(), StoreError> {
        if rate > 0 && self.at() == self.end() - 1 {
            self.seek(0)?;
        }
        self.set_rate(if rate < 0 && self.at() == 0 { 0 } else { rate });
        Ok(())
    }

    pub(super) fn step(&mut self, delta: i32) -> Result<(), StoreError> {
        self.set_rate(0);
        self.seek(self.at().saturating_add(delta))
    }

    pub(super) fn seek(&mut self, at: i32) -> Result<(), StoreError> {
        let at = at.clamp(0, self.end() - 1);
        if at == self.at() && !self.player.ended() {
            return Ok(());
        }
        self.player.seek(at).map_err(|error| viewer_error(&error))?;
        self.pending = Some(at);
        Ok(())
    }

    pub(super) fn scrub(&mut self, phase: Phase, at: i32) -> Result<(), StoreError> {
        match phase {
            Phase::Began => {
                self.scrub_origin = Some(self.at());
                self.set_rate(0);
                self.seek(at)
            }
            Phase::Changed => self.seek(at),
            Phase::Ended | Phase::Cancelled => {
                let origin = self.scrub_origin.take();
                if phase == Phase::Ended {
                    self.seek(at)?;
                } else if let Some(original) = origin {
                    self.seek(original)?;
                }
                Ok(())
            }
        }
    }

    pub(super) fn shown(&self) -> Option<i32> {
        self.shown
    }
}

fn shuttle_rate(rate: i8, direction: i8) -> i8 {
    let direction = direction.signum();
    if rate != 0 && rate.signum() == direction {
        (rate * 2).clamp(-8, 8)
    } else {
        direction
    }
}

#[cfg(test)]
#[test]
fn shuttle_doubles_in_one_direction_and_resets_when_reversed() {
    let mut rate = 0;
    for expected in [1, 2, 4, 8, 8] {
        rate = shuttle_rate(rate, 1);
        assert_eq!(rate, expected);
    }
    for expected in [-1, -2, -4, -8, -8] {
        rate = shuttle_rate(rate, -1);
        assert_eq!(rate, expected);
    }
    assert_eq!(shuttle_rate(rate, 0), 0);
    assert_eq!(shuttle_rate(rate, 1), 1);
}

pub(super) fn composition(store: &Store) -> Result<Option<Rc<Composition>>, StoreError> {
    let project = store.project();
    if project.clips.order().is_empty() && project.items.order().is_empty() {
        return Ok(None);
    }
    // Offline media shows as a labeled placeholder; the title counts it.
    match Composition::prepare_for_viewer(project, store.path()) {
        Ok(composition) => Ok(Some(Rc::new(composition))),
        Err(error) => Err(viewer_error(&error)),
    }
}

/// Program viewport state. Zoom and pan never modify the project canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct View {
    fitted: bool,
    scale: f32,
    pan: [f32; 2],
}

impl Default for View {
    fn default() -> Self {
        Self {
            fitted: true,
            scale: 1.0,
            pan: [0.0; 2],
        }
    }
}

impl View {
    pub(super) fn fitted(self) -> bool {
        self.fitted
    }

    pub(super) fn zoom_percent(self, stage: [f32; 2], canvas: [u32; 2]) -> u32 {
        (self.display_scale(stage, canvas) * 100.0).round() as u32
    }

    pub(super) fn display_scale(self, stage: [f32; 2], canvas: [u32; 2]) -> f32 {
        self.scale_for(stage, canvas.map(|n| n as f32))
    }

    pub(super) fn at_scale(scale: f32) -> Self {
        Self {
            fitted: false,
            scale: scale.clamp(0.01, 8.0),
            pan: [0.0; 2],
        }
    }

    pub(super) fn step_zoom(self, factor: f32, stage: [f32; 2], canvas: [u32; 2]) -> Self {
        let canvas = canvas.map(|value| value as f32);
        self.zoom(
            self.scale_for(stage, canvas) * factor,
            [0.0; 2],
            stage,
            canvas,
        )
    }

    pub(super) fn zoom_to(self, scale: f32, stage: [f32; 2], canvas: [u32; 2]) -> Self {
        self.zoom(scale, [0.0; 2], stage, canvas.map(|value| value as f32))
    }

    fn scale_for(self, stage: [f32; 2], canvas: [f32; 2]) -> f32 {
        if self.fitted {
            fit_scale(stage, canvas)
        } else {
            self.scale
        }
    }

    fn zoom(self, next: f32, anchor: [f32; 2], stage: [f32; 2], canvas: [f32; 2]) -> Self {
        let fit = fit_scale(stage, canvas);
        let scale = next.clamp(fit.min(0.05), 8.0);
        if (scale - fit).abs() < 0.002 {
            return Self::default();
        }
        let ratio = scale / self.scale_for(stage, canvas);
        let pan = [
            anchor[0] - (anchor[0] - self.pan[0]) * ratio,
            anchor[1] - (anchor[1] - self.pan[1]) * ratio,
        ];
        Self {
            fitted: false,
            scale,
            pan,
        }
        .clamped(stage, canvas)
    }

    fn clamped(mut self, [width, height]: [f32; 2], [x, y]: [f32; 2]) -> Self {
        let max_x = ((x * self.scale - width) * 0.5 + 40.0).max(0.0);
        let max_y = ((y * self.scale - height) * 0.5 + 40.0).max(0.0);
        self.pan = [
            self.pan[0].clamp(-max_x, max_x),
            self.pan[1].clamp(-max_y, max_y),
        ];
        self
    }

    fn destination(self, rect: Rect, [width, height]: [f32; 2]) -> Rect {
        let stage = [rect.w, rect.h];
        let canvas = [width, height];
        let scale = self.scale_for(stage, canvas);
        let [pan_x, pan_y] = self.clamped(stage, canvas).pan;
        let [w, h] = [width * scale, height * scale];
        Rect::new(
            rect.x + (rect.w - w) * 0.5 + pan_x,
            rect.y + (rect.h - h) * 0.5 + pan_y,
            w,
            h,
        )
    }
}

/// Renders the texture and returns the view, stage node, and context-menu position.
pub(super) fn paint(
    ui: &mut Ui,
    size: Option<[u32; 2]>,
    matte: Option<Image>,
    image: Option<Image>,
    mut view: View,
    events: &[input::Event],
    guide: Option<Guide>,
) -> (View, Handle, Option<Point>) {
    let viewport = ui.node(
        "program",
        Node {
            w: Size::Grow,
            h: Size::Grow,
            flags: Flags::CLIP | Flags::CLICKABLE | Flags::SCROLL | Flags::SCROLL_X,
            ..Node::DEFAULT
        },
    );
    let canvas = size
        .filter(|size| size[0] != 0 && size[1] != 0)
        .map(|size| size.map(|n| n as f32));
    let rect = ui.rect(&viewport);
    let signal = ui.signal(&viewport);
    if let Some(canvas) = canvas
        && rect.w > 0.0
        && rect.h > 0.0
    {
        view = interaction(ui, &signal, view, rect, canvas, events);
    }
    let context = (signal.pressed && signal.button == Some(Signal::BUTTON_SECONDARY))
        .then(|| Point::new(signal.press_point.0, signal.press_point.1));
    ui.canvas(&viewport, move |paint, rect| {
        if let Some(matte) = matte {
            paint
                .image(rect, &matte, &ImageStyle::DEFAULT.fit(Fit::Fill))
                .expect("valid stage matte");
        } else {
            paint.fill(rect, Fill::new(Rgba::rgb(6, 6, 7)));
        }
        if let (Some(canvas), Some(image)) = (canvas, image) {
            let destination = view.destination(rect, canvas);
            paint
                .image(destination, &image, &ImageStyle::DEFAULT)
                .expect("valid live program image");
            if let Some(guide) = guide {
                guide.paint(paint, destination);
            }
        }
    });
    (view, viewport, context)
}

fn interaction(
    ui: &Ui,
    signal: &Signal,
    mut view: View,
    rect: Rect,
    canvas: [f32; 2],
    events: &[input::Event],
) -> View {
    let stage = [rect.w, rect.h];
    let relative = |(x, y): (f32, f32)| [x - rect.x - rect.w * 0.5, y - rect.y - rect.h * 0.5];
    if signal.double_clicked {
        return if view.fitted {
            view.zoom(1.0, relative(signal.release_point), stage, canvas)
        } else {
            View::default()
        };
    }

    let mut precise = None;
    if signal.hovered {
        for event in events {
            if let input::Event::TrackpadScrolled { delta, phase, .. } = event {
                let (dx, dy) = precise.get_or_insert((0.0, 0.0));
                if matches!(
                    phase,
                    input::ScrollPhase::Touched | input::ScrollPhase::Moved
                ) {
                    *dx += delta.0;
                    *dy += delta.1;
                }
            }
        }
    }
    let ([dx, dy], points) = if let Some((x, y)) = precise {
        ([x as f32, y as f32], true)
    } else {
        ([signal.scroll.0, signal.scroll.1], false)
    };
    let modified =
        (dx != 0.0 || dy != 0.0) && (ui.key_down(KeyCode::Meta) || ui.key_down(KeyCode::Ctrl));
    let pinch = signal.pinch as f32;
    let mut factor = if pinch.is_finite() && pinch > 0.0 {
        pinch
    } else {
        1.0
    };
    if modified {
        factor *= (dy * if points { 0.01 } else { 0.05 }).exp();
    }
    if factor != 1.0 {
        view = view.zoom(
            view.scale_for(stage, canvas) * factor,
            relative(signal.pointer),
            stage,
            canvas,
        );
    }
    if !view.fitted {
        if !modified {
            let step = if points { 1.0 } else { 24.0 };
            view.pan[0] += dx * step;
            view.pan[1] += dy * step;
        }
        if signal.button == Some(pxui::Signal::BUTTON_PRIMARY) {
            view.pan[0] += signal.drag.0;
            view.pan[1] += signal.drag.1;
        }
        view = view.clamped(stage, canvas);
    }
    view
}

fn fit_scale([width, height]: [f32; 2], [canvas_width, canvas_height]: [f32; 2]) -> f32 {
    ((width - 24.0) / canvas_width)
        .min((height - 24.0) / canvas_height)
        .max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_rect_preserves_portrait_and_landscape_shapes() {
        let view = View::default();
        let window = Rect::new(10.0, 30.0, 144.0, 104.0);
        assert_eq!(
            view.destination(window, [1080.0, 1920.0]),
            Rect::new(59.5, 42.0, 45.0, 80.0)
        );
        assert_eq!(
            view.destination(window, [1920.0, 960.0]),
            Rect::new(22.0, 52.0, 120.0, 60.0)
        );
    }

    #[test]
    fn stage_zoom_keeps_pointer_anchor_and_limits_pan() {
        let view = View::default();
        let stage = [500.0, 300.0];
        let canvas = [1000.0, 1000.0];
        let scale = view.scale_for(stage, canvas);
        let next = view.zoom(scale * 2.0, [30.0, -20.0], stage, canvas);
        assert!(!next.fitted());
        assert_eq!(next.pan, [-30.0, 20.0]);
        assert_eq!(next.zoom_percent(stage, [1000, 1000]), 55);
        let far = View {
            pan: [10000.0, -10000.0],
            ..next
        }
        .clamped(stage, canvas);
        assert!((far.pan[0] - 66.0).abs() < 0.001);
        assert!((far.pan[1] + 166.0).abs() < 0.001);
    }

    #[test]
    fn stage_handles_double_click_and_pinch_via_pxui() {
        use pxui::{Event, testing};
        use std::cell::Cell;

        let current = Cell::new(View::default());
        let context = Cell::new(None);
        let build = &mut |ui: &mut Ui| {
            let (view, _, point) =
                paint(ui, Some([1080, 1920]), None, None, current.get(), &[], None);
            current.set(view);
            if point.is_some() {
                context.set(point);
            }
        };
        let mut ui = pxui::doc::frames(2, build);
        testing::click(&mut ui, 160.0, 120.0, build);
        testing::click(&mut ui, 160.0, 120.0, build);
        assert_eq!(
            current.get().zoom_percent([320.0, 240.0], [1080, 1920]),
            100
        );
        testing::frame(
            &mut ui,
            &[Event {
                kind: Event::PINCHED,
                value: 0.5,
                ..Event::default()
            }],
            build,
        );
        assert_eq!(current.get().zoom_percent([320.0, 240.0], [1080, 1920]), 50);
        testing::frame(
            &mut ui,
            &[Event::pointer(
                Event::POINTER_PRESSED,
                u32::from(Signal::BUTTON_SECONDARY),
                42.0,
                55.0,
            )],
            build,
        );
        assert_eq!(context.get(), Some(Point::new(42.0, 55.0)));
    }
}
