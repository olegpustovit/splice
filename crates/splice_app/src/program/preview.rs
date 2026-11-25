/// v13 Program texture size from a logical canvas, displayed scale and window pixel density.
pub(super) fn size(canvas: [u32; 2], scale: f32, dpr: f32) -> [u32; 2] {
    let longest = u64::from(canvas[0].max(canvas[1]).max(1));
    let displayed = longest as f64 * f64::from(scale) * f64::from(dpr);
    let edge = displayed.round().clamp(320.0, 1920.0) as u64;
    canvas.map(|side| ((u64::from(side) * edge + longest / 2) / longest).max(1) as u32)
}

/// One active size. Coalesce changing layout measurements before allocating a new set of textures.
#[derive(Default)]
pub(super) struct Raster {
    current: Option<[u32; 2]>,
    pending: Option<([u32; 2], f64)>,
}

impl Raster {
    pub(super) fn update(
        &mut self,
        requested: Option<[u32; 2]>,
        initial: [u32; 2],
        now: f64,
    ) -> Option<[u32; 2]> {
        let desired = requested.or(self.current).unwrap_or(initial);
        if self.current == Some(desired) {
            self.pending = None;
            return None;
        }
        if self.current.is_some() {
            let pending = self.pending.get_or_insert((desired, now));
            if pending.0 != desired {
                *pending = (desired, now);
            }
            if now < pending.1 + 0.075 {
                return None;
            }
        }
        self.current = Some(desired);
        self.pending = None;
        Some(desired)
    }

    pub(super) fn settling(&self) -> bool {
        self.pending.is_some()
    }
}
