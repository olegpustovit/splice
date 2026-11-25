use splice_eval::{FramePlan, Layer};
use splice_model::Rect;

/// Projects evaluated logical geometry into the live Program texture. Source UVs and clocks
/// remain on the original plan.
#[derive(Clone, Copy)]
pub(super) struct Projection {
    pub(super) size: [u32; 2],
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) short: f64,
}

impl Projection {
    pub(super) fn new(canvas: [u32; 2], size: [u32; 2]) -> Option<Self> {
        (canvas != size).then(|| Self {
            size,
            x: f64::from(size[0]) / f64::from(canvas[0]),
            y: f64::from(size[1]) / f64::from(canvas[1]),
            short: f64::from(size[0].min(size[1])) / f64::from(canvas[0].min(canvas[1])),
        })
    }

    pub(super) fn box_pixels(self, [x, y, w, h]: [f64; 4]) -> [f64; 4] {
        [x * self.x, y * self.y, w * self.x, h * self.y]
    }

    fn offset(self, offset: &mut [f64; 2]) {
        offset[0] *= self.x;
        offset[1] *= self.y;
    }

    fn rect(self, bounds: &mut Rect) {
        [bounds.x, bounds.y, bounds.w, bounds.h] =
            self.box_pixels([bounds.x, bounds.y, bounds.w, bounds.h]);
    }

    pub(super) fn frame(self, plan: &mut FramePlan) {
        plan.size = self.size;
        if let Some(shift) = &mut plan.shift {
            self.offset(&mut shift.offset);
        }
        if let Some(group) = &mut plan.group {
            self.offset(&mut group.offset);
            group.clip = group.clip.map(|clip| self.box_pixels(clip));
        }
        for layer in &mut plan.layers {
            match layer {
                Layer::Background { bounds, .. } => self.rect(bounds),
                Layer::Picture(picture) => {
                    self.rect(&mut picture.destination);
                    self.offset(&mut picture.look.offset);
                }
                Layer::Camera { bounds, radius, .. } => {
                    self.rect(bounds);
                    *radius *= self.short;
                }
                Layer::Shape {
                    bounds,
                    stroke_width,
                    corner,
                    look,
                    ..
                } => {
                    self.rect(bounds);
                    *stroke_width *= self.short;
                    *corner *= self.short;
                    self.offset(&mut look.offset);
                }
                Layer::Title {
                    bounds, font, look, ..
                } => {
                    self.rect(bounds);
                    *font *= self.short;
                    self.offset(&mut look.offset);
                }
                Layer::Caption { look, .. } => self.offset(&mut look.offset),
                Layer::Keystrokes { .. } | Layer::Fade { .. } => {}
            }
        }
    }
}
