use super::{surface::Surface, viewer_error};
use pxui::{Fill, Image, ImageStyle, Node, Rect, Rgba, Ui, ui::Size};
use splice_av::{Composition, Player};
use splice_core::history::{Store, StoreError};
use splice_kit::Phase;
use std::rc::Rc;

pub(super) struct Program {
    composition: Rc<Composition>,
    player: Player,
    shown: Option<i32>,
    pending: Option<i32>,
    playing: bool,
    scrub_origin: Option<(i32, bool)>,
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
            playing: false,
            scrub_origin: None,
        }))
    }

    pub(super) fn size(&self) -> [u32; 2] {
        self.composition.size()
    }

    pub(super) fn sample(&mut self, surface: &mut Surface) -> Result<(), StoreError> {
        if let Some(at) = surface.sample(&self.composition, &mut self.player)? {
            self.shown = Some(at);
            if self.pending == Some(at) || self.playing {
                self.pending = None;
            }
        }
        if self.playing && self.player.ended() {
            self.playing = false;
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
        self.playing
    }

    pub(super) fn dragging(&self) -> bool {
        self.scrub_origin.is_some()
    }

    pub(super) fn wants_frame(&self) -> bool {
        self.playing || self.pending.is_some() || self.shown.is_none()
    }

    pub(super) fn play(&mut self, playing: bool) {
        self.player.set_playing(playing);
        self.playing = playing;
    }

    pub(super) fn toggle(&mut self) -> Result<(), StoreError> {
        if !self.playing && self.at() == self.end() - 1 {
            self.seek(0)?;
        }
        self.play(!self.playing);
        Ok(())
    }

    pub(super) fn step(&mut self, delta: i32) -> Result<(), StoreError> {
        self.play(false);
        self.seek(self.at() + delta)
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
                self.scrub_origin = Some((self.at(), self.playing));
                self.play(false);
                self.seek(at)
            }
            Phase::Changed => self.seek(at),
            Phase::Ended | Phase::Cancelled => {
                let origin = self.scrub_origin.take();
                if phase == Phase::Ended {
                    self.seek(at)?;
                } else if let Some((original, _)) = origin {
                    self.seek(original)?;
                }
                if let Some((_, true)) = origin {
                    self.play(true);
                }
                Ok(())
            }
        }
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
    match Composition::prepare(project, store.path()) {
        Ok(composition) => Ok(Some(Rc::new(composition))),
        // Open without a picture; the title names the offline media.
        Err(splice_av::Error::Offline(_)) => Ok(None),
        Err(error) => Err(viewer_error(&error)),
    }
}

/// The program image, letterboxed; titles and captions are in it (drawn by export's rasters).
pub(super) fn paint(ui: &mut Ui, size: Option<[u32; 2]>, image: Option<Image>) {
    let viewport = ui.node(
        "program",
        Node {
            w: Size::Grow,
            h: Size::Grow,
            ..Node::DEFAULT
        },
    );
    ui.canvas(&viewport, move |paint, rect| {
        paint.fill(rect, Fill::new(Rgba::BLACK));
        if let (Some(size), Some(image)) = (size, image) {
            let [w, h] = size.map(|n| n as f32);
            let destination = letterbox(rect, [w, h]);
            if destination.w <= 0.0 || destination.h <= 0.0 {
                return;
            }
            paint
                .image(destination, &image, &ImageStyle::DEFAULT)
                .expect("valid live program image");
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
