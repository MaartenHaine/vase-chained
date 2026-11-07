//! Which keys the daemon is taking (`Daemon::key_mode`), and `modal` as a view of it.

use super::fake::*;
use crate::daemon::KeyMode;
use crate::geometry::Rect;
use crate::input::{InputCommand, Key, KeyCode, Mods};

/// `n` windows on one display, so `n` tabs.
fn tabs(n: u64) -> Shared {
    world(vec![display(1, 0.0, 0.0, 1000.0, 800.0)], (1..=n).map(|i| window(i, "App", Rect::new(10.0, 10.0, 400.0, 300.0))).collect())
}

fn shift(code: KeyCode) -> Key {
    Key { code, mods: Mods { shift: true, ..Mods::default() } }
}

fn press(d: &mut TestDaemon, code: KeyCode) {
    d.intercept_key(Key::plain(code));
}

#[test]
fn a_fresh_daemon_is_idle() {
    let w = tabs(2);
    let d = daemon(&w);
    assert_eq!(d.key_mode(), KeyMode::Idle);
    assert!(!d.modal());
}

#[test]
fn an_armed_prefix_is_the_routers_not_an_overlay() {
    let w = tabs(2);
    let mut d = daemon(&w);
    d.prefix_armed = true;
    assert_eq!(d.key_mode(), KeyMode::Prefix);
    assert!(!d.modal(), "the router takes the next key, so intercept_key must not swallow it");
}

#[test]
fn a_half_typed_tab_number_is_a_prefix_that_intercept_key_takes() {
    let w = tabs(12);
    let mut d = daemon(&w);
    d.run(InputCommand::SelectBarTab(1));
    assert_eq!(d.key_mode(), KeyMode::Prefix);
    assert!(d.modal());
    press(&mut d, KeyCode::Escape);
    assert_eq!(d.key_mode(), KeyMode::Idle);
}

#[test]
fn the_sheet_the_switcher_and_the_command_line_are_overlays() {
    let w = tabs(2);
    let mut d = daemon(&w);
    d.toggle_help();
    assert_eq!(d.key_mode(), KeyMode::Overlay);
    press(&mut d, KeyCode::Char('x')); // any key closes the sheet
    assert_eq!(d.key_mode(), KeyMode::Idle);

    d.run(InputCommand::WindowSwitcher);
    assert_eq!(d.key_mode(), KeyMode::Overlay);
    press(&mut d, KeyCode::Escape);
    assert_eq!(d.key_mode(), KeyMode::Idle);

    d.run(InputCommand::CommandLine);
    assert_eq!(d.key_mode(), KeyMode::Overlay);
    assert!(d.modal());
    press(&mut d, KeyCode::Escape);
    assert_eq!(d.key_mode(), KeyMode::Idle);
}

#[test]
fn the_picker_is_an_overlay_and_a_launch_from_it_is_pending() {
    let w = tabs(1);
    w.borrow_mut().apps = vec!["Calculator".into()];
    let mut d = daemon(&w);
    d.run(InputCommand::SplitH); // the new empty pane opens the picker
    assert!(d.pane_picker.is_some());
    assert_eq!(d.key_mode(), KeyMode::Overlay);

    for c in "/calc".chars() {
        press(&mut d, KeyCode::Char(c));
    }
    press(&mut d, KeyCode::Return);
    assert!(d.pending_launch.is_some());
    assert_eq!(d.key_mode(), KeyMode::PendingLaunch);
    assert!(d.modal());

    press(&mut d, KeyCode::Escape); // cancels the launch and closes the pane
    assert_eq!(d.key_mode(), KeyMode::Idle);
}

#[test]
fn a_resize_leaves_exactly_its_repeat_keys_sticky() {
    let w = tabs(1);
    let mut d = daemon(&w);
    d.run(InputCommand::ResizeLeft);
    let KeyMode::Sticky(keys) = d.key_mode() else { panic!("expected Sticky, got {:?}", d.key_mode()) };
    let codes = [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down, KeyCode::Char('h'), KeyCode::Char('j'), KeyCode::Char('k'), KeyCode::Char('l')];
    assert_eq!(keys.len(), codes.len());
    for code in codes {
        assert!(keys.contains(&shift(code)), "{code:?} with Shift repeats a resize");
    }
    assert!(!d.modal(), "other keys still reach the app");
    // Every listed key really does repeat, and keeps the mode.
    for key in keys {
        assert!(d.intercept_key(key), "{key:?} is consumed as a repeat");
        assert!(matches!(d.key_mode(), KeyMode::Sticky(_)));
    }
    // Any other key ends it and is not consumed.
    assert!(!d.intercept_key(Key::plain(KeyCode::Char('x'))));
    assert_eq!(d.key_mode(), KeyMode::Idle);
}

#[test]
fn a_move_tab_leaves_shift_comma_and_shift_period_sticky() {
    let w = tabs(3);
    let mut d = daemon(&w);
    d.run(InputCommand::MoveTabNext);
    assert_eq!(d.key_mode(), KeyMode::Sticky(vec![shift(KeyCode::Char(',')), shift(KeyCode::Char('.'))]));
    assert!(d.intercept_key(shift(KeyCode::Char('.'))));
    // Without Shift the same key is not a repeat: it ends the mode and reaches the app.
    assert!(!d.intercept_key(Key::plain(KeyCode::Char('.'))));
    assert_eq!(d.key_mode(), KeyMode::Idle);
}

#[test]
fn another_prefix_command_leaves_sticky() {
    let w = tabs(1);
    let mut d = daemon(&w);
    d.run(InputCommand::ResizeLeft);
    d.run(InputCommand::ZoomToggle);
    assert_eq!(d.key_mode(), KeyMode::Idle);
}
