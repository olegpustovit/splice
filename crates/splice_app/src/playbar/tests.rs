use super::*;
use pxui::{Event, Role, UiDesc, testing};
use std::cell::Cell;

#[test]
fn playbar_controls_have_accessible_names_and_cancel_captured_scrubs() {
    let view = Cell::new(View {
        at: 15,
        end: 61,
        fps: 30,
        play: CommandState::default(),
        rate: 0,
        dragging: false,
    });
    let result = Cell::new(Intents::default());
    let mut build = |ui: &mut Ui| result.set(paint(ui, view.get()));
    let mut ui = pxui::doc::frames(1, &mut build);
    let play = testing::find(&ui, Role::Button, "Play").unwrap();
    let slider = testing::find(&ui, Role::Slider, "Playhead").unwrap();
    testing::click(
        &mut ui,
        f64::from(play.x + play.w * 0.5),
        f64::from(play.y + play.h * 0.5),
        &mut build,
    );
    assert!(result.take().play);
    let input = |ui: &mut Ui, kind, fraction| {
        testing::frame(
            ui,
            &[Event {
                kind,
                x: f64::from(slider.x + slider.w * fraction),
                y: f64::from(slider.y + slider.h * 0.5),
                ..Event::default()
            }],
            &mut build,
        );
    };
    let mut input = input;
    input(&mut ui, Event::POINTER_PRESSED, 0.25);
    assert_eq!(result.take().scrub, Some((Phase::Began, 15)));
    view.set(View {
        dragging: true,
        ..view.get()
    });
    input(&mut ui, Event::POINTER_MOVED, 0.75);
    assert_eq!(result.take().scrub, Some((Phase::Changed, 45)));
    input(&mut ui, Event::FOCUS_LOST, 0.75);
    assert_eq!(result.take().scrub, Some((Phase::Cancelled, 15)));
    assert_eq!(timecode(108_017, 30), [1, 0, 0, 17]);
}

#[test]
fn playbar_running_clock_allocates_nothing_with_warmed_digits() {
    mem_sys::set_peer(pxui::library_counter);
    let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
    let view = View {
        at: 0,
        end: 61,
        fps: 30,
        play: CommandState {
            title: Some("Pause"),
            ..CommandState::default()
        },
        rate: 1,
        dragging: false,
    };
    let mut tick = || {
        for at in 0..20 {
            testing::frame(&mut ui, &[], &mut |ui| {
                paint(ui, View { at, ..view });
            });
        }
    };
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
        .warm(&mut tick);
}
