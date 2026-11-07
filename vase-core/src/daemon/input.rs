//! Running a bound command, and the sticky repeat modes that outlive the prefix.

use crate::focus::Direction;
use crate::input::{InputCommand, Key, KeyCode, Mods};
use crate::model::Command;

use super::Daemon;
use crate::backend::Backend;
use crate::chrome::Painter;

/// The bare repeat keys after a prefix resize, each with Shift: arrows and vim keys both, matching
/// the prefix bindings.
const RESIZE_REPEAT: [(KeyCode, Direction); 8] = [
    (KeyCode::Left, Direction::Left),
    (KeyCode::Char('h'), Direction::Left),
    (KeyCode::Right, Direction::Right),
    (KeyCode::Char('l'), Direction::Right),
    (KeyCode::Up, Direction::Up),
    (KeyCode::Char('k'), Direction::Up),
    (KeyCode::Down, Direction::Down),
    (KeyCode::Char('j'), Direction::Down),
];

/// The bare repeat keys after a prefix move-tab, each with Shift, and which way they move the tab.
const MOVETAB_REPEAT: [(KeyCode, isize); 2] = [(KeyCode::Char(','), -1), (KeyCode::Char('.'), 1)];

/// Which keys the daemon is taking right now, for a platform that has to grab keys before they
/// arrive rather than decide on each one as it comes. Follows the precedence of `intercept_key`,
/// then the prefix router, then the sticky repeat keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMode {
    /// Nothing is in progress: only the prefix chords and the app-focus hotkeys are the daemon's.
    Idle,
    /// The next key is the daemon's: a prefix is armed (as the platform last reported it in
    /// `prefix_armed`), or a `prefix-<number>` is waiting for another digit.
    Prefix,
    /// An overlay takes every key: the shortcut sheet, the command line, the window switcher or the
    /// pane picker.
    Overlay,
    /// After a prefix resize or move-tab, these keys (each with Shift only) repeat it. Any other key
    /// belongs to the focused app and ends the mode.
    Sticky(Vec<Key>),
    /// A launch into the focused empty pane is waiting for its window. Every key is taken, and only
    /// Esc acts: it cancels the launch and closes the pane.
    PendingLaunch,
}

impl<B: Backend, C: Painter> Daemon<B, C> {
    /// Which keys the daemon is taking right now. See `KeyMode`.
    pub fn key_mode(&self) -> KeyMode {
        if self.help_open {
            return KeyMode::Overlay;
        }
        if self.pending_launch.is_some() {
            return KeyMode::PendingLaunch;
        }
        if self.prompt.is_some() || self.switcher.is_some() || self.pane_picker.is_some() {
            return KeyMode::Overlay;
        }
        if self.tab_entry.is_pending() || self.prefix_armed {
            return KeyMode::Prefix;
        }
        let shift = Mods { shift: true, ..Mods::default() };
        let mut keys: Vec<Key> = Vec::new();
        if self.resize_sticky {
            keys.extend(RESIZE_REPEAT.iter().map(|(code, _)| Key { code: *code, mods: shift }));
        }
        if self.movetab_sticky {
            keys.extend(MOVETAB_REPEAT.iter().map(|(code, _)| Key { code: *code, mods: shift }));
        }
        if keys.is_empty() {
            KeyMode::Idle
        } else {
            KeyMode::Sticky(keys)
        }
    }

    /// Whether an overlay is taking keys: whether `intercept_key` will swallow the next one. For a
    /// platform that has to answer "is this key the overlay's?" at a moment when it cannot ask the
    /// daemon itself.
    pub fn modal(&self) -> bool {
        match self.key_mode() {
            KeyMode::Overlay | KeyMode::PendingLaunch => true,
            // A half-typed tab number is `intercept_key`'s; an armed prefix is the router's.
            KeyMode::Prefix => self.tab_entry.is_pending(),
            KeyMode::Idle | KeyMode::Sticky(_) => false,
        }
    }

    /// Offer a key to whatever is modal, before the prefix router sees it. Returns whether the key was swallowed. The order here *is* the modal precedence.
    pub fn intercept_key(&mut self, key: Key) -> bool {
        // The sheet is a read-only overlay, so the next key of any kind dismisses it and is spent doing so.
        if self.help_open {
            self.close_help();
            return true;
        }
        // A launch in flight makes its pane modal: only Esc, which cancels and collapses the pane.
        if self.pending_launch.is_some() {
            if key.code == KeyCode::Escape {
                self.pending_launch = None;
                self.dispatch(Command::CloseFocusedPane);
            }
            return true;
        }
        if self.prompt.is_some() {
            return self.prompt_key(key);
        }
        if self.switcher.is_some() {
            return self.switcher_key(key);
        }
        if self.pane_picker.is_some() {
            return self.pane_picker_key(key);
        }
        // Mid `prefix-<number>` entry: capture further digits (the router only routes the first key after the prefix) so multi-digit tab numbers work.
        if self.tab_entry.is_pending() {
            return self.tab_entry_key(key);
        }
        // Configurable global app-focus hotkeys (e.g. Ctrl-` → Ghostty). Direct chords, not behind the prefix; the modal overlays above take precedence.
        if let Some(app) = self.app_hotkey(key) {
            self.toggle_app_focus(&app);
            return true;
        }
        // After a prefix resize or move-tab, the bare repeat keys keep working without re-prefixing.
        self.sticky_key(key)
    }

    /// Run one bound command. Model edits go through the reducer; the rest drive the daemon itself.
    pub fn run(&mut self, cmd: InputCommand) {
        use InputCommand as I;
        // A resize / move-tab arms its sticky mode; any other prefix command leaves both.
        self.resize_sticky = matches!(cmd, I::ResizeLeft | I::ResizeRight | I::ResizeUp | I::ResizeDown);
        self.movetab_sticky = matches!(cmd, I::MoveTabPrev | I::MoveTabNext);
        if let Some(c) = Command::from_input(&cmd) {
            self.dispatch(c);
            return;
        }
        match cmd {
            // tmux last-window: jump back to the previously-focused window.
            I::LastTab => {
                if let Some(w) = self.last_focused {
                    self.dispatch(Command::Raise(w));
                }
            }
            I::Quit => {
                self.restore();
                self.quit = true;
            }
            I::WindowSwitcher => self.open_switcher(),
            I::Rename => self.start_rename(),
            I::StackRename => self.start_stack_rename(),
            I::CommandLine => self.start_command(),
            I::WarpCursor => self.warp_cursor_to_focus(),
            I::Resync => self.resync(),
            I::Help => self.toggle_help(),
            I::SelectBarTab(n) => self.begin_tab_entry(n),
            I::SendPrefix => {}
            // Everything else is a model edit `from_input` already returned.
            _ => {}
        }
    }

    /// Move the mouse cursor to the center of the focused pane.
    fn warp_cursor_to_focus(&self) {
        if let Some(r) = self.model.as_ref().and_then(|m| m.focused_pane_rect()) {
            self.backend.warp_cursor(r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
    }

    /// After a prefix resize or move-tab, the bare repeat keys keep acting until any other key. Returns whether the key was consumed.
    pub fn sticky_key(&mut self, key: Key) -> bool {
        let shift_only = key.mods.shift_only();
        if self.resize_sticky {
            let dir = RESIZE_REPEAT.iter().find(|(code, _)| *code == key.code).map(|(_, dir)| *dir);
            if let (true, Some(dir)) = (shift_only, dir) {
                self.dispatch(Command::Resize(dir));
                return true;
            }
            self.resize_sticky = false;
        }
        if self.movetab_sticky {
            let offset = MOVETAB_REPEAT.iter().find(|(code, _)| *code == key.code).map(|(_, offset)| *offset);
            if let (true, Some(offset)) = (shift_only, offset) {
                self.dispatch(Command::MoveTab(offset));
                return true;
            }
            self.movetab_sticky = false;
        }
        false
    }

    /// End a sticky resize or move-tab the way any other key would, for a platform that cannot see
    /// that other key and ends the mode on a timer instead. Returns whether one was on.
    pub fn end_sticky(&mut self) -> bool {
        let was = self.resize_sticky || self.movetab_sticky;
        self.resize_sticky = false;
        self.movetab_sticky = false;
        was
    }
}
