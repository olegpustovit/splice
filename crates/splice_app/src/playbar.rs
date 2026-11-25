#[cfg(test)]
use pxui::{
    Button, Fill, Label, Node, Rect, Scrubber, Stroke, Ui,
    geom::Edges,
    ui::{Align, Size},
};
use splice_kit::CommandState;
#[cfg(test)]
use splice_kit::Phase;

#[derive(Clone, Copy)]
pub(super) struct View {
    pub at: i32,
    pub end: i32,
    pub fps: u32,
    pub play: CommandState,
    pub rate: i8,
    pub dragging: bool,
}

pub(super) fn clock(at: i32, fps: u32) -> [u32; 3] {
    let at = at.max(0) as u32;
    let fps = fps.max(1);
    let seconds = at / fps;
    [seconds / 60, seconds % 60, at % fps]
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct Intents {
    pub play: bool,
    pub scrub: Option<(Phase, i32)>,
}

#[cfg(test)]
pub(super) fn paint(ui: &mut Ui, view: View) -> Intents {
    let mut intents = Intents::default();
    ui.row(
        Node {
            align: Align::Center,
            padding: Edges::all(8.0),
            gap: 12.0,
            ..Node::DEFAULT
        },
        |ui| {
            let play = Button::DEFAULT
                .size(Size::Px(64.0), Size::Px(30.0))
                .centered()
                .disabled(!view.play.enabled);
            intents.play = ui
                .button("play", view.play.title.unwrap_or("Play"), &play)
                .clicked;
            let last = view.end.saturating_sub(1).max(0);
            let scrub = ui.scrub(
                "playhead",
                view.at as f32,
                0.0..=last as f32,
                &Scrubber::DEFAULT.step(1.0).name("Playhead"),
            );
            let handle = scrub.response.handle;
            ui.set_node_layout(&handle, &Node::DEFAULT.h_px(30.0));
            let at = scrub
                .preview
                .map_or(view.at, |value| value.round() as i32)
                .clamp(0, last);
            if let Some(phase) =
                Phase::from_interaction(view.dragging, scrub.response.held, scrub.commit.is_some())
            {
                intents.scrub = Some((phase, at));
            }
            let fraction = if last > 0 {
                at as f32 / last as f32
            } else {
                0.0
            };
            let tokens = ui.tokens();
            let focused = scrub.response.focused;
            ui.canvas(&handle, move |paint, rect| {
                if focused {
                    paint.stroke(rect, Stroke::new(tokens.accent));
                }
                let y = rect.y + rect.h * 0.5;
                let thumb = 4.0_f32.min(rect.w * 0.5);
                let x = rect.x + thumb + fraction * (rect.w - 2.0 * thumb);
                paint.fill(
                    Rect::new(rect.x, y - 2.0, rect.w, 4.0),
                    Fill::new(tokens.track),
                );
                if last > 0 {
                    paint.fill(
                        Rect::new(rect.x, y - 2.0, x - rect.x, 4.0),
                        Fill::new(tokens.accent),
                    );
                    paint.circle((x, y), thumb, Some(Fill::new(tokens.accent)), None);
                }
            });
            let [hours, minutes, seconds, frames] = timecode(at, view.fps);
            ui.label_digits(
                format_args!("{hours:02}:{minutes:02}:{seconds:02}:{frames:02}"),
                Label::MONO,
            );
        },
    );
    intents
}

#[cfg(test)]
fn timecode(at: i32, fps: u32) -> [u32; 4] {
    let [minutes, seconds, frames] = clock(at, fps);
    [minutes / 60, minutes % 60, seconds, frames]
}

#[cfg(test)]
mod tests;
