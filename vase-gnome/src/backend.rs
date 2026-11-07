//! `GnomeBackend`: vase-core's `Backend` over the mirror (protocol §6.1). Reads come from the mirror;
//! each call that changes something is queued as an effect for the next `effects` line and recorded
//! in the mirror at once, so the reads later in the same dispatch already see it.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use serde_json::{json, Value};
use vase_core::backend::{Backend, Display, WindowInfo};
use vase_core::chrome::Position;
use vase_core::geometry::Rect;
use vase_core::tree::WindowId;

use crate::mirror::Mirror;
use crate::painter::Surfaces;
use crate::protocol::{rect, wire_rect, WireWindow};

/// What the backend and the painter share with the host: the mirror, the effects not yet sent, and
/// the chrome as it will next be drawn. vase-core owns the backend and the painter once the daemon
/// is built, so the host keeps a handle to this instead.
#[derive(Default)]
pub struct Shared {
    pub mirror: Mirror,
    /// Effects emitted since the last `effects` line, in call order.
    pub effects: Vec<Value>,
    pub surfaces: Surfaces,
}

pub type SharedRef = Rc<RefCell<Shared>>;

pub struct GnomeBackend(pub SharedRef);

fn info(w: &WireWindow) -> WindowInfo {
    WindowInfo { id: WindowId(w.id), pid: w.pid, app: w.app.clone(), title: w.title.clone(), frame: w.frame.map(rect).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)), layer: w.layer }
}

impl GnomeBackend {
    fn emit(&self, effect: Value) {
        self.0.borrow_mut().effects.push(effect);
    }
}

impl Backend for GnomeBackend {
    fn displays(&self) -> Vec<Display> {
        self.0.borrow().mirror.core_displays()
    }

    /// The windows on the active workspace (or on every workspace), not minimized: what vase calls
    /// on screen. A minimized window keeps its tab through `minimized`.
    fn list_windows(&mut self) -> Vec<WindowInfo> {
        self.0.borrow().mirror.stack().iter().filter(|w| w.on_workspace && !w.minimized).map(info).collect()
    }

    fn all_windows(&mut self) -> Vec<WindowInfo> {
        self.0.borrow().mirror.stack().iter().map(info).collect()
    }

    /// A placement the extension already reports as done is not sent again: re-sending a frame the
    /// window has is a no-op on screen, and leaving it out is what keeps a restart or a reconnect from
    /// touching any window.
    fn set_frame(&mut self, window: WindowId, frame: Rect) {
        let mut shared = self.0.borrow_mut();
        if !shared.mirror.placing(window.0) && shared.mirror.reported_frame(window.0) == Some(frame) {
            return;
        }
        shared.mirror.set_frame(window.0, frame);
        shared.effects.push(json!({"k": "place", "id": window.0, "rect": wire_rect(frame)}));
    }

    fn focus(&mut self, window: WindowId) {
        self.0.borrow_mut().mirror.set_focus(window.0);
        self.emit(json!({"k": "focus", "id": window.0}));
    }

    fn raise(&mut self, window: WindowId) {
        self.0.borrow_mut().mirror.raise(window.0);
        self.emit(json!({"k": "raise", "id": window.0}));
    }

    fn forget(&mut self, _window: WindowId) {}

    fn title(&mut self, window: WindowId) -> Option<String> {
        self.0.borrow().mirror.get(window.0).map(|w| w.title)
    }

    fn tileable(&mut self, info: &WindowInfo) -> Option<bool> {
        self.0.borrow().mirror.get(info.id.0).map(|w| w.tileable)
    }

    fn minimized(&mut self, window: WindowId) -> Option<bool> {
        self.0.borrow().mirror.get(window.0).map(|w| w.minimized)
    }

    fn minimized_info(&mut self, info: &WindowInfo) -> Option<bool> {
        self.minimized(info.id)
    }

    fn set_minimized(&mut self, window: WindowId, minimized: bool) {
        self.0.borrow_mut().mirror.set_minimized(window.0, minimized);
        self.emit(json!({"k": "minimize", "id": window.0, "minimized": minimized}));
    }

    fn fullscreen(&mut self, info: &WindowInfo) -> Option<bool> {
        self.0.borrow().mirror.get(info.id.0).map(|w| w.fullscreen)
    }

    fn close(&mut self, window: WindowId) {
        self.emit(json!({"k": "close", "id": window.0}));
    }

    fn launchable_apps(&self) -> Vec<String> {
        self.0.borrow().mirror.apps.iter().map(|a| a.app.clone()).collect()
    }

    /// Launch by desktop-file id; an app with none known is not launched (the extension would refuse
    /// it anyway, protocol §6.1).
    fn launch(&self, app: &str) {
        let id = self.0.borrow().mirror.app_id(app);
        match id {
            Some(app_id) => self.emit(json!({"k": "launch", "app_id": app_id})),
            None => eprintln!("vase-gnome: no desktop-file id for {app:?}; not launching it"),
        }
    }

    fn warp_cursor(&self, x: f64, y: f64) {
        self.emit(json!({"k": "warp", "at": [x, y]}));
    }

    fn badged_apps(&self) -> HashSet<String> {
        HashSet::new()
    }

    fn activate(&self, app: &str) {
        let id = self.0.borrow().mirror.app_id(app);
        if let Some(app_id) = id {
            self.emit(json!({"k": "activate", "app_id": app_id}));
        }
    }

    /// The Ubuntu Dock autohides at the bottom, so the bar goes at the top on Linux (owner, Q1.5).
    fn default_bar_position(&self) -> Position {
        Position::Top
    }

    /// GNOME's primary monitor, which can sit anywhere in the layout (the owner's is at x=2560, F11).
    fn primary_display(&self, _displays: &[Display]) -> usize {
        self.0.borrow().mirror.primary
    }
}
