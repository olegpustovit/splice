use pxui::{
    Point, Ui,
    ui::{Menu, MenuItem},
};

const WIDTH: f32 = 208.0;
const ZOOM_CHIP_OFFSET: f32 = 83.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Fit,
    In,
    Out,
    Percent(u32),
    Crop,
}

pub(super) struct Intents {
    pub open: bool,
    pub action: Option<Action>,
}

pub(super) fn paint(
    ui: &mut Ui,
    viewer_width: f32,
    window_height: f32,
    scale: f32,
    fitted: bool,
    context: Option<Point>,
) -> Intents {
    let bounds = ui.content_rect();
    let width = (bounds.w - 8.0).clamp(1.0, WIDTH);
    let left = bounds.x + 4.0;
    let right = (bounds.x + bounds.w - width - 4.0).max(left);
    let top = bounds.y + 4.0;
    let bottom = (bounds.y + bounds.h - 4.0).max(top);
    let position = context.unwrap_or(Point::new(viewer_width - ZOOM_CHIP_OFFSET, 62.0));
    let position = Point::new(position.x.clamp(left, right), position.y.clamp(top, bottom));
    let is_scale = |preset: f32| !fitted && (scale - preset).abs() < 0.001;
    let menu = Menu::DEFAULT
        .w_px(width)
        .row_h(22.0)
        .padding(4.0)
        .max_h((window_height - 8.0).max(1.0))
        .focus_on_click(true)
        .name("Viewer zoom");
    let items = [
        MenuItem::new("Fit").checked(fitted).shortcut("⇧Z").id(0),
        MenuItem::new("Zoom In").shortcut("⌘=").id(1),
        MenuItem::new("Zoom Out").shortcut("⌘−").id(2),
        MenuItem::new(""),
        MenuItem::new("25%").checked(is_scale(0.25)).id(25),
        MenuItem::new("50%").checked(is_scale(0.5)).id(50),
        MenuItem::new("100%").checked(is_scale(1.0)).id(100),
        MenuItem::new("200%").checked(is_scale(2.0)).id(200),
        MenuItem::new("400%").checked(is_scale(4.0)).id(400),
        MenuItem::new(""),
        MenuItem::new("Crop Clip at Playhead…")
            .shortcut("⇧C")
            .id(500),
    ];
    let picked = ui.menu("viewer-zoom-menu", position, &items, &menu);
    let action = match picked.chosen_id {
        Some(0) => Some(Action::Fit),
        Some(1) => Some(Action::In),
        Some(2) => Some(Action::Out),
        Some(500) => Some(Action::Crop),
        Some(value) => Some(Action::Percent(value as u32)),
        None => None,
    };
    Intents {
        open: !picked.dismissed && action.is_none(),
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{Role, testing};
    use std::cell::Cell;

    #[test]
    fn warmed_zoom_menu_paints_without_allocations() {
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = pxui::Ui::new(&pxui::UiDesc::DEFAULT).unwrap();
        let mut tick = || {
            for _ in 0..10 {
                testing::frame(&mut ui, &[], &mut |ui| {
                    let result = paint(ui, 320.0, 240.0, 0.5, false, None);
                    assert!(result.open && result.action.is_none());
                });
            }
        };
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut tick);
    }
    #[test]
    fn zoom_menu_returns_selected_percentage_and_dismisses() {
        let intent = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            let result = paint(ui, 320.0, 240.0, 0.5, false, None);
            if let Some(action) = result.action {
                assert!(!result.open);
                intent.set(Some(action));
            }
        };
        let mut ui = pxui::doc::frames(2, &mut draw);
        let choice = testing::find(&ui, Role::MenuItem, "200%").unwrap();
        testing::click(
            &mut ui,
            f64::from(choice.x + choice.w * 0.5),
            f64::from(choice.y + choice.h * 0.5),
            &mut draw,
        );
        assert_eq!(intent.get(), Some(Action::Percent(200)));
    }

    #[test]
    fn zoom_context_menu_uses_real_scale_and_stays_inside_window() {
        let scale = Cell::new(0.254);
        let mut draw = |ui: &mut Ui| {
            paint(
                ui,
                320.0,
                240.0,
                scale.get(),
                false,
                Some(Point::new(320.0, 238.0)),
            );
        };
        let mut ui = pxui::doc::frames(2, &mut draw);
        let flags = |ui: &Ui| {
            ui.a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::MenuItem) && ui.a11y_name_of(node) == "25%")
                .unwrap()
                .states
        };
        assert_eq!(flags(&ui) & pxui::A11yStates::CHECKED.bits(), 0);
        let rect = testing::find(&ui, Role::MenuItem, "25%").unwrap();
        assert!(rect.x >= 0.0 && rect.x + rect.w <= ui.view_size().width);
        scale.set(0.25);
        testing::frame(&mut ui, &[], &mut draw);
        assert_ne!(flags(&ui) & pxui::A11yStates::CHECKED.bits(), 0);
    }
}
