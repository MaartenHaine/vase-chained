//! Ending the prefix from outside the key stream (`Daemon::cancel_prefix`).

use super::fake::*;
use crate::geometry::Rect;
use crate::input::InputCommand;

/// `n` windows on one display, so `n` tabs: twelve make `prefix-1` wait for a second digit.
fn tabs(n: u64) -> Shared {
    world(vec![display(1, 0.0, 0.0, 1000.0, 800.0)], (1..=n).map(|i| window(i, "App", Rect::new(10.0, 10.0, 400.0, 300.0))).collect())
}

#[test]
fn cancel_drops_a_half_typed_tab_number_without_selecting() {
    let w = tabs(12);
    let mut d = daemon(&w);
    let before = d.model.as_ref().unwrap().screens[0].current;
    d.run(InputCommand::SelectBarTab(1));
    assert!(d.tab_entry.is_pending(), "1 could still become 10, 11 or 12");
    assert!(d.cancel_prefix());
    assert!(!d.tab_entry.is_pending());
    assert!(!d.modal());
    // Nothing was selected, and the next digit starts a new number.
    assert_eq!(d.model.as_ref().unwrap().screens[0].current, before);
    d.run(InputCommand::SelectBarTab(1));
    assert!(d.tab_entry.is_pending());
}

#[test]
fn cancel_clears_the_prefix_dot() {
    let w = tabs(2);
    let mut d = daemon(&w);
    d.prefix_armed = true;
    d.refresh();
    assert!(w.borrow().bar_armed);
    assert!(d.cancel_prefix());
    assert!(!d.prefix_armed);
    assert!(!w.borrow().bar_armed, "the bar is redrawn without the dot");
}

#[test]
fn cancel_with_nothing_pending_changes_nothing() {
    let w = tabs(2);
    let mut d = daemon(&w);
    assert!(!d.cancel_prefix());
    assert!(w.borrow().bar.is_none(), "no redraw when there was no dot to clear");
}
