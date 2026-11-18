use super::{surface::Surface, viewer_error};
use pxui::{
    Fill, Image, ImageStyle, Label, Node, Point, Rect, Rgba, TextAlign, TextSpan, Ui, ui::Size,
};
use splice_av::{Composition, Player};
use splice_core::history::{Store, StoreError};
use std::rc::Rc;

pub(super) struct Program {
    composition: Rc<Composition>,
    player: Player,
    shown: Option<i32>,
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
        }))
    }

    pub(super) fn size(&self) -> [u32; 2] {
        self.composition.size()
    }

    pub(super) fn sample(&mut self, surface: &mut Surface) -> Result<(), StoreError> {
        if let Some(at) = surface.sample(&mut self.player)? {
            self.shown = Some(at);
        }
        Ok(())
    }

    pub(super) fn shown(&self) -> Option<i32> {
        self.shown
    }

    pub(super) fn contents(&self) -> Option<(i32, &Rc<Composition>)> {
        self.shown.map(|at| (at, &self.composition))
    }
}

pub(super) fn composition(store: &Store) -> Result<Option<Rc<Composition>>, StoreError> {
    let project = store.project();
    if project.clips.order().is_empty() && project.items.order().is_empty() {
        return Ok(None);
    }
    Composition::prepare(project, store.path())
        .map(|composition| Some(Rc::new(composition)))
        .map_err(|error| viewer_error(&error))
}

pub(super) fn paint(ui: &mut Ui, contents: Option<(i32, &Rc<Composition>)>, image: Option<Image>) {
    let viewport = ui.node(
        "program",
        Node {
            w: Size::Grow,
            h: Size::Grow,
            ..Node::DEFAULT
        },
    );
    let contents = contents.map(|(at, composition)| (at, Rc::clone(composition)));
    ui.canvas(&viewport, move |paint, rect| {
        paint.fill(rect, Fill::new(Rgba::BLACK));
        if let (Some((at, clip)), Some(image)) = (&contents, image) {
            let [w, h] = clip.size().map(|n| n as f32);
            let destination = letterbox(rect, [w, h]);
            if destination.w <= 0.0 || destination.h <= 0.0 {
                return;
            }
            paint
                .image(destination, &image, &ImageStyle::DEFAULT)
                .expect("valid live program image");
            let scale = destination.w / w;
            for cue in clip.text_at(*at) {
                let layer = &cue.layer;
                let [x, y, width, _] = layer.bounds.map(|value| value as f32 * scale);
                let alpha = (cue.opacity(*at).clamp(0.0, 1.0) * 255.0).round() as u8;
                if alpha == 0 || layer.text.is_empty() {
                    continue;
                }
                let rgb = layer.color.to_be_bytes();
                let color = Rgba::rgba(rgb[1], rgb[2], rgb[3], alpha);
                let span = TextSpan::new(0, layer.text.len() as u32).color(color);
                let style = Label::BODY
                    .size((layer.font as f32 * scale).max(1.0))
                    .bold()
                    .wrap()
                    .align(TextAlign::Center);
                paint
                    .text_spans(
                        Point::new(destination.x + x, destination.y + y),
                        style,
                        width,
                        &layer.text,
                        &[span],
                    )
                    .expect("valid program text span");
            }
        }
    });
}

pub(super) fn letterbox(rect: Rect, [width, height]: [f32; 2]) -> Rect {
    let scale = (rect.w / width).min(rect.h / height);
    let [w, h] = [scale * width, scale * height];
    Rect::new(
        rect.x + (rect.w - w) * 0.5,
        rect.y + (rect.h - h) * 0.5,
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_rect_preserves_portrait_and_landscape_shapes() {
        let window = Rect::new(10.0, 30.0, 120.0, 80.0);
        assert_eq!(
            letterbox(window, [1080.0, 1920.0]),
            Rect::new(47.5, 30.0, 45.0, 80.0)
        );
        assert_eq!(
            letterbox(window, [1920.0, 960.0]),
            Rect::new(10.0, 40.0, 120.0, 60.0)
        );
    }
}
