//! The daemon driven whole, over a fake platform.

mod fake;
mod key_mode;
mod prefix;
mod primary;
mod sticky;

use crate::geometry::Rect;
use crate::input::InputCommand;
use crate::model::Effect;
use crate::tree::WindowId;

use fake::*;

/// One 1000x800 display at the origin with two windows on it.
fn two_windows() -> Shared {
    world(vec![display(1, 0.0, 0.0, 1000.0, 800.0)], vec![window(1, "Editor", Rect::new(10.0, 10.0, 400.0, 300.0)), window(2, "Terminal", Rect::new(50.0, 50.0, 400.0, 300.0))])
}

#[test]
fn a_render_places_every_window_of_the_current_tab() {
    let w = two_windows();
    let mut d = daemon(&w);
    let placements = d.model.as_ref().unwrap().placements();
    d.execute(vec![Effect::Render(placements.clone())]);
    assert_eq!(w.borrow().frames, placements);
}

#[test]
fn a_quiet_reconcile_moves_nothing() {
    let w = two_windows();
    let mut d = daemon(&w);
    d.reconcile();
    assert!(w.borrow().frames.is_empty());
}

#[test]
fn the_tab_bar_is_drawn_on_the_main_display_and_shows_the_prefix_dot() {
    let w = two_windows();
    let mut d = daemon(&w);
    d.refresh();
    let bar = w.borrow().bar.expect("a bar is drawn");
    assert_eq!((bar.x, bar.w), (0.0, 1000.0));
    assert!(!w.borrow().bar_armed);
    d.prefix_armed = true;
    d.refresh();
    assert!(w.borrow().bar_armed);
}

#[test]
fn the_sheet_and_the_switcher_show_on_the_painter() {
    let w = two_windows();
    let mut d = daemon(&w);
    d.toggle_help();
    assert!(w.borrow().help);
    d.toggle_help();
    assert!(!w.borrow().help);
    d.run(InputCommand::WindowSwitcher);
    assert!(w.borrow().list);
    assert!(d.switcher.is_some());
}

#[test]
fn focus_fronts_the_window_in_the_fake() {
    let w = two_windows();
    let mut d = daemon(&w);
    d.execute(vec![Effect::FocusWindow(WindowId(2))]);
    assert_eq!(w.borrow().windows[0].id, WindowId(2));
}
