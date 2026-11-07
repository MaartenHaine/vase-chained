//! Ending a sticky repeat from outside the key stream (`Daemon::end_sticky`).

use super::fake::*;
use crate::geometry::Rect;
use crate::input::{InputCommand, Key, KeyCode, Mods};

/// `n` windows on one display, so `n` tabs.
fn tabs(n: u64) -> Shared {
    world(vec![display(1, 0.0, 0.0, 1000.0, 800.0)], (1..=n).map(|i| window(i, "App", Rect::new(10.0, 10.0, 400.0, 300.0))).collect())
}

fn shift(code: KeyCode) -> Key {
    Key { code, mods: Mods { shift: true, ..Mods::default() } }
}

#[test]
fn ending_a_sticky_resize_hands_the_repeat_keys_back_to_the_app() {
    let w = tabs(1);
    let mut d = daemon(&w);
    d.run(InputCommand::ResizeLeft);
    assert!(d.intercept_key(shift(KeyCode::Left)), "still repeating before the end");
    assert!(d.end_sticky());
    assert!(!d.intercept_key(shift(KeyCode::Left)), "the key passes through once the mode ended");
}

#[test]
fn ending_a_sticky_move_tab_leaves_the_tab_where_it_is() {
    let w = tabs(3);
    let mut d = daemon(&w);
    d.run(InputCommand::MoveTabNext);
    let after_move = d.model.clone();
    assert!(d.end_sticky());
    assert_eq!(d.model, after_move, "ending the mode is not a move");
    assert!(!d.intercept_key(shift(KeyCode::Char('.'))));
    assert_eq!(d.model, after_move);
}

#[test]
fn ending_when_nothing_is_sticky_reports_so() {
    let w = tabs(1);
    let mut d = daemon(&w);
    assert!(!d.end_sticky());
    d.run(InputCommand::ResizeLeft);
    assert!(d.end_sticky());
    assert!(!d.end_sticky(), "only the first end finds it on");
}

#[test]
fn a_new_resize_after_the_end_is_sticky_again() {
    let w = tabs(1);
    let mut d = daemon(&w);
    d.run(InputCommand::ResizeLeft);
    d.end_sticky();
    d.run(InputCommand::ResizeRight);
    assert!(d.intercept_key(shift(KeyCode::Char('l'))));
}
