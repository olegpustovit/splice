use super::super::{Parity, Selection, paint_spec, tests::paint_spec_titlebar};
use super::DIAGRAM_NAME;
use pxui::{Event, FrameIn, Node, Rect, Role, Size, Ui, testing};

const TITLES: &[&str] = &["Architecture", "UI framework: PXUI", "Icon system"];

#[test]
fn engineering_spec_960() {
    engineering_spec(960.0);
}

#[test]
fn engineering_spec_800() {
    engineering_spec(800.0);
}

#[test]
fn engineering_spec_480() {
    engineering_spec(480.0);
}

fn engineering_spec(width: f32) {
    let mut ui = crate::viewer_ui().unwrap();
    ui.set_a11y(true);
    let input = FrameIn::new(width, 640.0);
    let mut selection = Selection::default();
    for _ in 0..2 {
        draw(&mut ui, &input, &mut selection);
    }
    mem_sys::set_peer(pxui::library_counter);
    for (index, &title) in TITLES.iter().enumerate() {
        jump(&mut ui, &input, &mut selection, width, Role::Button, title);
        let heading = geometry(&ui, width, title, Role::Heading, title);
        let expected_x = if width > 820.0 { 252.0 } else { 16.0 };
        let expected_y = if width > 820.0 { 55.0 } else { 21.0 };
        assert!((heading.x - expected_x).abs() < 0.1, "{heading:?}");
        assert!((heading.y - expected_y).abs() < 0.15, "{heading:?}");
        let content_width = width - expected_x - if width > 820.0 { 32.0 } else { 16.0 };

        match index {
            0 => {
                let diagram = geometry(&ui, width, title, Role::Image, DIAGRAM_NAME);
                assert!((diagram.w - content_width).abs() < 0.15, "{diagram:?}");
                assert!(
                    (diagram.h - content_width * 470.0 / 960.0).abs() < 0.15,
                    "{diagram:?}"
                );
                for table in ["Crates", "Threads"] {
                    let table = geometry(&ui, width, title, Role::Table, table);
                    assert!((table.w - content_width).abs() < 0.15, "{table:?}");
                }
                geometry(&ui, width, title, Role::ScrollArea, "Project package");
                geometry(&ui, width, title, Role::Heading, "Undo in Rust");
            }
            1 => {
                let capabilities = geometry(&ui, width, title, Role::Table, "PXUI capabilities");
                assert!(
                    (capabilities.w - content_width).abs() < 0.15,
                    "{capabilities:?}"
                );
                let commands = geometry(&ui, width, title, Role::Group, "PXUI Commands");
                let undo = geometry(&ui, width, title, Role::Group, "PXUI Undo");
                if width > 820.0 {
                    assert!((undo.x - commands.x - commands.w - 12.0).abs() < 0.15);
                    assert_eq!((undo.y, undo.h), (commands.y, commands.h));
                } else {
                    assert_eq!((undo.x, undo.w), (commands.x, commands.w));
                    assert!((undo.y - commands.y - commands.h - 12.0).abs() < 0.15);
                }
                let abi = pxui::PxuiUiDesc::default().abi_built;
                geometry(
                    &ui,
                    width,
                    title,
                    Role::Heading,
                    &format!(
                        "Where PXUI stands (pinned ABI {}.{})",
                        abi >> 16,
                        abi & 0xffff
                    ),
                );
            }
            _ => {
                let next = testing::find(&ui, Role::Heading, "Media on macOS (no ffmpeg)").unwrap();
                let glyphs = ui
                    .a11y_nodes()
                    .iter()
                    .filter(|node| {
                        node.is_role(Role::Image) && node.rect.y > heading.y && node.rect.y < next.y
                    })
                    .count();
                assert_eq!(glyphs, 57);
                let first = geometry(&ui, width, title, Role::Image, "Go to start");
                let last = geometry(&ui, width, title, Role::Image, "Go to end");
                let columns = ((content_width + 6.0) / 90.0).floor();
                let tile = (content_width - (columns - 1.0) * 6.0) / columns;
                assert!((first.w - 22.0).abs() < 0.15, "{first:?}");
                if columns >= 6.0 {
                    assert_eq!(last.y, first.y);
                    assert!((last.x - first.x - 5.0 * (tile + 6.0)).abs() < 0.15);
                } else {
                    assert_eq!(last.x, first.x);
                    assert!((last.y - first.y - 66.0).abs() < 0.15);
                }
                let mark =
                    testing::find(&ui, Role::Image, "Splice mark: a cut point on a pulse").unwrap();
                let ink = ui
                    .layers()
                    .flat_map(|layer| layer.instances)
                    .filter(|instance| {
                        instance.kind == pxui::draw::kind::CURVE
                            && instance.rect[0] >= mark.x
                            && instance.rect[2] <= mark.x + mark.w
                            && instance.rect[1] >= mark.y
                            && instance.rect[3] <= mark.y + mark.h
                    })
                    .count();
                assert!(
                    ink >= 5,
                    "the brand cut emits its five native curve segments ({ink})"
                );
                for (role, name) in [
                    (Role::Image, "Splice mark: a cut point on a pulse"),
                    (Role::Image, "Color"),
                    (Role::Image, "Close"),
                    (Role::Link, "UI framework: PXUI"),
                ] {
                    geometry(&ui, width, title, role, name);
                }
            }
        }
        let idle = &mut || draw(&mut ui, &input, &mut selection);
        for _ in 0..8 {
            idle();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(idle);
        println!("WARM|{width}|{title}|0 allocations");
    }
    jump(
        &mut ui,
        &input,
        &mut selection,
        width,
        Role::Link,
        "UI framework: PXUI",
    );
    let heading = testing::find(&ui, Role::Heading, "UI framework: PXUI").unwrap();
    assert!(
        (heading.y - if width > 820.0 { 55.0 } else { 21.0 }).abs() < 0.15,
        "{heading:?}"
    );
}

pub(super) fn draw(ui: &mut Ui, input: &FrameIn, selection: &mut Selection) {
    let mut next = None;
    ui.frame(input, &mut |ui| {
        ui.column(Node::column().w(Size::Grow).h(Size::Grow), |ui| {
            next = paint_spec(
                ui,
                *selection,
                (&[], &Parity::default()),
                &mut paint_spec_titlebar,
            )
            .or(next);
        });
    })
    .unwrap();
    if let Some(next) = next {
        *selection = next;
    }
}

fn jump(
    ui: &mut Ui,
    input: &FrameIn,
    selection: &mut Selection,
    width: f32,
    role: Role,
    name: &str,
) {
    let find = |ui: &Ui| testing::find(ui, role, name).unwrap_or_else(|| panic!("missing {name}"));
    if role == Role::Button && width <= 820.0 && selection.scroll > 0.0 {
        draw(
            ui,
            &input.events(&[
                Event::pointer(Event::POINTER_MOVED, 0, (width - 12.0).into(), 290.0),
                Event::pointer(
                    Event::SCROLLED,
                    0,
                    0.0,
                    f64::from(selection.scroll / 24.0 + 1.0),
                ),
            ]),
            selection,
        );
        draw(ui, input, selection);
    }
    let rect = find(ui);
    if role == Role::Button {
        let x = rect.x + rect.w * 0.5;
        let y = rect.y + rect.h * 0.5;
        if width <= 820.0 && !(6.0..width - 6.0).contains(&x) {
            draw(
                ui,
                &input.events(&[
                    Event::pointer(Event::POINTER_MOVED, 0, 120.0, 48.0),
                    Event::pointer(Event::SCROLLED, 0, f64::from((width * 0.5 - x) / 24.0), 0.0),
                ]),
                selection,
            );
        } else if width > 820.0 && y > 625.0 {
            draw(
                ui,
                &input.events(&[
                    Event::pointer(Event::POINTER_MOVED, 0, 120.0, 300.0),
                    Event::pointer(Event::SCROLLED, 0, 0.0, f64::from((300.0 - y) / 24.0)),
                ]),
                selection,
            );
        }
    } else {
        let y = rect.y + rect.h * 0.5;
        if !(80.0..625.0).contains(&y) {
            draw(
                ui,
                &input.events(&[
                    Event::pointer(Event::POINTER_MOVED, 0, f64::from(width - 24.0), 290.0),
                    Event::pointer(Event::SCROLLED, 0, 0.0, f64::from((300.0 - y) / 24.0)),
                ]),
                selection,
            );
        }
    }
    draw(ui, input, selection);
    let rect = find(ui);
    let x = rect.x + rect.w * 0.5;
    let y = rect.y + rect.h * 0.5;
    assert!(
        (0.0..width).contains(&x) && (0.0..640.0).contains(&y),
        "{name}: {rect:?}"
    );
    for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
        draw(
            ui,
            &input.events(&[Event::pointer(kind, 0, x.into(), y.into())]),
            selection,
        );
    }
    draw(ui, input, selection);
}

fn geometry(ui: &Ui, width: f32, section: &str, role: Role, name: &str) -> Rect {
    let rect = testing::find(ui, role, name).unwrap_or_else(|| panic!("missing {name}"));
    println!(
        "GEOM|{width}|{section}|{name}|{}|{}|{}|{}",
        rect.x, rect.y, rect.w, rect.h
    );
    rect
}
