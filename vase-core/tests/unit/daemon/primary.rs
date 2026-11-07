//! Which display is the main one, where the tab bar lives (`Backend::primary_display`).

use super::fake::*;
use crate::chrome::{usable, Position};
use crate::geometry::Rect;

/// A 4K monitor at scale 1.5 (2560x1440 logical) at the origin, and a 1920x1080 monitor to its
/// right at x=2560, with a window on each.
fn side_by_side(primary: Option<usize>) -> Shared {
    let w = world(
        vec![display(1, 0.0, 0.0, 2560.0, 1440.0), display(2, 2560.0, 0.0, 1920.0, 1080.0)],
        vec![window(1, "Editor", Rect::new(100.0, 100.0, 800.0, 600.0)), window(2, "Terminal", Rect::new(2700.0, 100.0, 800.0, 600.0))],
    );
    w.borrow_mut().primary = primary;
    w
}

fn bar_x(w: &Shared) -> f64 {
    w.borrow().bar.expect("a bar is drawn").x
}

#[test]
fn by_default_the_display_at_the_origin_gets_the_tab_bar() {
    let w = side_by_side(None);
    let mut d = daemon(&w);
    d.refresh();
    assert_eq!(bar_x(&w), 0.0);
    d.reconcile();
    assert_eq!(bar_x(&w), 0.0, "a quiet reconcile keeps it there");
    assert!(w.borrow().frames.is_empty(), "and moves nothing");
}

#[test]
fn a_primary_at_x_2560_gets_the_tab_bar_and_gives_up_its_strip() {
    let w = side_by_side(Some(1));
    let mut d = daemon(&w);
    d.refresh();
    assert_eq!(bar_x(&w), 2560.0);
    assert_eq!(w.borrow().bar.unwrap().w, 1920.0);
    let screens = &d.model.as_ref().unwrap().screens;
    assert_eq!(screens[1].rect, usable(Rect::new(2560.0, 0.0, 1920.0, 1080.0), true, Position::Bottom));
    assert_eq!(screens[0].rect, Rect::new(0.0, 0.0, 2560.0, 1440.0), "the other display keeps its whole work area");
    // The poll agrees with startup, so nothing is re-tiled.
    d.reconcile();
    assert!(w.borrow().frames.is_empty());
    assert_eq!(bar_x(&w), 2560.0);
}

#[test]
fn a_primary_that_moves_with_no_display_changing_moves_the_tab_bar() {
    let w = side_by_side(Some(1));
    let mut d = daemon(&w);
    d.refresh();
    w.borrow_mut().primary = Some(0);
    d.reconcile();
    assert_eq!(bar_x(&w), 0.0);
    let screens = &d.model.as_ref().unwrap().screens;
    assert_eq!(screens[0].rect, usable(Rect::new(0.0, 0.0, 2560.0, 1440.0), true, Position::Bottom));
    assert_eq!(screens[1].rect, Rect::new(2560.0, 0.0, 1920.0, 1080.0));
    // Both tabs were re-tiled onto the new rects.
    assert!(!w.borrow().frames.is_empty());
}

#[test]
fn an_index_past_the_last_display_falls_back_to_the_origin() {
    let w = side_by_side(Some(1));
    let mut d = daemon(&w);
    d.refresh();
    w.borrow_mut().primary = Some(7);
    d.reconcile();
    assert_eq!(bar_x(&w), 0.0);
}
