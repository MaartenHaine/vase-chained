//! Letting go of the screen without undoing the layout (`Daemon::detach`).

use std::path::PathBuf;

use super::fake::*;
use crate::daemon::Paths;
use crate::geometry::Rect;
use crate::input::{InputCommand, Key, KeyCode, Mods};
use crate::model::Effect;
use crate::tree::WindowId;

fn two_windows() -> Shared {
    world(vec![display(1, 0.0, 0.0, 1000.0, 800.0)], vec![window(1, "Editor", Rect::new(10.0, 10.0, 400.0, 300.0)), window(2, "Terminal", Rect::new(50.0, 50.0, 400.0, 300.0))])
}

/// The daemon after its first render, with the record of that render cleared.
fn rendered(w: &Shared, paths: Paths) -> TestDaemon {
    let mut d = daemon_with(w, paths);
    let placements = d.model.as_ref().unwrap().placements();
    d.execute(vec![Effect::Render(placements)]);
    d.refresh();
    w.borrow_mut().frames.clear();
    d
}

/// A state file of this test's own, in a directory removed when it is dropped.
struct StateDir(PathBuf);

impl StateDir {
    fn new(test: &str) -> StateDir {
        let dir = std::env::temp_dir().join(format!("vase-core-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        StateDir(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("state.json")
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn detach_hides_every_surface_and_moves_no_window() {
    let w = two_windows();
    let mut d = rendered(&w, Paths::default());
    d.toggle_help();
    assert!(w.borrow().bar.is_some() && w.borrow().help);
    d.detach();
    let world = w.borrow();
    assert!(world.bar.is_none() && !world.help && !world.list, "nothing is left on screen");
    assert!(world.frames.is_empty(), "no window is put back where it was before vase");
}

#[test]
fn detach_saves_the_layout() {
    let dir = StateDir::new("detach-saves");
    let w = two_windows();
    let mut d = daemon_with(&w, Paths { config: None, state: Some(dir.file()) });
    assert!(!dir.file().exists(), "nothing saved before the first dispatch");
    d.detach();
    let (model, windows) = crate::state::load(&dir.file()).expect("the layout was saved");
    assert_eq!(Some(model), d.model);
    assert_eq!(windows.len(), 2);
}

#[test]
fn detach_drops_whatever_was_half_typed() {
    let w = two_windows();
    let mut d = rendered(&w, Paths::default());
    d.run(InputCommand::CommandLine);
    d.intercept_key(Key::plain(KeyCode::Char('q')));
    d.detach();
    assert!(d.prompt.is_none(), "a half-typed :q never runs");
    assert!(!d.modal());

    d.run(InputCommand::WindowSwitcher);
    d.detach();
    assert!(d.switcher.is_none());

    d.prefix_armed = true;
    d.run(InputCommand::ResizeLeft);
    d.detach();
    assert!(!d.prefix_armed);
    let shift_left = Key { code: KeyCode::Left, mods: Mods { shift: true, ..Mods::default() } };
    assert!(!d.intercept_key(shift_left), "the sticky resize ended");
}

#[test]
fn a_resume_redraws_and_a_later_quit_still_restores() {
    let w = two_windows();
    let mut d = rendered(&w, Paths::default());
    d.detach();
    d.refresh(); // the host is back
    assert!(w.borrow().bar.is_some());
    assert!(w.borrow().frames.is_empty());
    d.restore();
    let frames = w.borrow().frames.clone();
    assert!(frames.contains(&(WindowId(1), Rect::new(10.0, 10.0, 400.0, 300.0))));
    assert!(frames.contains(&(WindowId(2), Rect::new(50.0, 50.0, 400.0, 300.0))));
}

#[test]
fn detaching_twice_is_harmless() {
    let w = two_windows();
    let mut d = rendered(&w, Paths::default());
    let model = d.model.clone();
    d.detach();
    d.detach();
    assert_eq!(d.model, model);
    assert!(w.borrow().frames.is_empty());
}
