//! The daemon's copy of the compositor's window state (protocol §6.1).
//!
//! vase's `Backend` reads are synchronous, and they happen inside a dispatch, straight after the
//! effects that change them (`tab_partly_covered` lists the windows right after a focus), so the
//! daemon can't ask the extension and wait. It reads this mirror instead. The mirror is optimistic:
//! each effect the daemon emits is recorded as an op and shown at once, and the extension's own
//! reports (the snapshot, deltas, `settled`) are kept apart as the truth. An op lasts until a report
//! that follows it arrives: a raise, a focus or a minimize until a line whose `ack` names its batch
//! or a later one; a placement until a `settled` for it, or a `window` delta that carries a frame
//! and acknowledges the batch. So a delta sent before the extension applied an effect never undoes
//! it, and once the extension has caught up the mirror shows exactly what it reported.

use std::collections::{HashMap, HashSet};

use vase_core::geometry::Rect;

use crate::protocol::{rect, WireApp, WireDisplay, WireRect, WireWindow};

/// The batch of an op whose effects line hasn't been sent yet: newer than any `ack`.
pub const UNSENT: u64 = u64::MAX;

/// The largest resize increment the daemon allows for, per axis, in stage pixels: an `x11` window can
/// only take sizes in its increments (S4: an xterm 2-7 px short at 9x18, a 2x xterm up to 13 px short
/// at 18x35), and the extension doesn't report them, so this bounds the §6.2 tolerance instead.
pub const X11_MAX_INC: (f64, f64) = (48.0, 48.0);

#[derive(Debug, Clone, Copy, PartialEq)]
enum OpKind {
    Frame(Rect),
    /// A raise, and a focus (which raises on GNOME too).
    Raise,
    Focus,
    Minimize(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Op {
    batch: u64,
    window: u64,
    kind: OpKind,
}

#[derive(Default)]
pub struct Mirror {
    /// What the extension last reported, by window id.
    windows: HashMap<u64, WireWindow>,
    /// Window ids frontmost first, as last reported.
    stacking: Vec<u64>,
    focus: Option<u64>,
    pub displays: Vec<WireDisplay>,
    pub primary: usize,
    pub apps: Vec<WireApp>,
    /// The daemon's own effects not yet confirmed, oldest first.
    ops: Vec<Op>,
}

impl Mirror {
    /// Replace everything with a snapshot. Every op is dropped: the snapshot is newer than all of them.
    pub fn load(&mut self, displays: Vec<WireDisplay>, primary: usize, windows: Vec<WireWindow>, focus: Option<u64>, apps: Vec<WireApp>) {
        self.stacking = windows.iter().map(|w| w.id).collect();
        self.windows = windows.into_iter().map(|w| (w.id, w)).collect();
        self.focus = focus;
        self.displays = displays;
        self.primary = primary;
        self.apps = apps;
        self.ops.clear();
    }

    /// The extension has finished applying every batch up to `ack`.
    pub fn ack(&mut self, ack: u64) {
        self.ops.retain(|op| matches!(op.kind, OpKind::Frame(_)) || op.batch > ack);
    }

    /// A `window` delta. A window not seen before goes in front, where mutter maps a new window,
    /// until a `stacking` delta says otherwise.
    pub fn window(&mut self, mut w: WireWindow, ack: u64) {
        self.ack(ack);
        let id = w.id;
        match w.frame {
            Some(_) => self.ops.retain(|op| !(op.window == id && matches!(op.kind, OpKind::Frame(_)) && op.batch <= ack)),
            // In flight: the frame stays what it was.
            None => w.frame = self.windows.get(&id).and_then(|old| old.frame),
        }
        if !self.windows.contains_key(&id) {
            self.stacking.insert(0, id);
        }
        self.windows.insert(id, w);
    }

    pub fn window_gone(&mut self, id: u64, ack: u64) {
        self.ack(ack);
        self.windows.remove(&id);
        self.stacking.retain(|w| *w != id);
        self.ops.retain(|op| op.window != id);
        if self.focus == Some(id) {
            self.focus = None;
        }
    }

    pub fn stacking(&mut self, ids: Vec<u64>, ack: u64) {
        self.ack(ack);
        self.stacking = ids;
    }

    pub fn focus(&mut self, id: Option<u64>, ack: u64) {
        self.ack(ack);
        self.focus = id;
    }

    pub fn displays(&mut self, displays: Vec<WireDisplay>, primary: usize, ack: u64) {
        self.ack(ack);
        self.displays = displays;
        self.primary = primary;
    }

    /// A placement from batch `re` settled at `frame`.
    pub fn settled(&mut self, id: u64, re: u64, frame: WireRect, ack: u64) {
        self.ack(ack);
        if let Some(w) = self.windows.get_mut(&id) {
            w.frame = Some(frame);
        }
        self.ops.retain(|op| !(op.window == id && matches!(op.kind, OpKind::Frame(_)) && op.batch <= re));
    }

    /// Record the daemon's own effects as they are emitted; `seal` gives them their batch.
    pub fn set_frame(&mut self, id: u64, frame: Rect) {
        self.ops.push(Op { batch: UNSENT, window: id, kind: OpKind::Frame(frame) });
    }

    pub fn raise(&mut self, id: u64) {
        self.ops.push(Op { batch: UNSENT, window: id, kind: OpKind::Raise });
    }

    pub fn set_focus(&mut self, id: u64) {
        self.ops.push(Op { batch: UNSENT, window: id, kind: OpKind::Focus });
    }

    pub fn set_minimized(&mut self, id: u64, minimized: bool) {
        self.ops.push(Op { batch: UNSENT, window: id, kind: OpKind::Minimize(minimized) });
    }

    /// The ops just emitted went out as effects line `batch`.
    pub fn seal(&mut self, batch: u64) {
        for op in self.ops.iter_mut().filter(|op| op.batch == UNSENT) {
            op.batch = batch;
        }
    }

    /// Forget the ops whose effects were never sent: the connection went before they could be.
    pub fn drop_unsent(&mut self) {
        self.ops.retain(|op| op.batch != UNSENT);
    }

    /// Whether a placement of `id` is still unconfirmed.
    pub fn placing(&self, id: u64) -> bool {
        self.ops.iter().any(|op| op.window == id && matches!(op.kind, OpKind::Frame(_)))
    }

    /// The windows with a placement the extension never confirmed. Read before a `load`, it names the
    /// placements a disconnect cut short: unread in the socket, held behind one in flight, or waiting
    /// for its re-send (§6.2).
    pub fn unfinished(&self) -> HashSet<u64> {
        self.ops.iter().filter(|op| matches!(op.kind, OpKind::Frame(_))).map(|op| op.window).collect()
    }

    /// Whether the extension already reports `id` at `frame`, as §6.2 accepts a placement: exactly, or
    /// for an `x11` window at the same origin and short by less than one resize increment per axis
    /// (never larger). Ignores the daemon's own placements.
    pub fn placed_as(&self, id: u64, frame: Rect) -> bool {
        let Some(w) = self.windows.get(&id) else { return false };
        let Some(got) = w.frame.map(rect) else { return false };
        if got == frame {
            return true;
        }
        let short = |want: f64, have: f64, inc: f64| have <= want && want - have < inc;
        w.x11 && got.x == frame.x && got.y == frame.y && short(frame.w, got.w, X11_MAX_INC.0) && short(frame.h, got.h, X11_MAX_INC.1)
    }

    /// A window as the daemon should see it now: its report with the unconfirmed ops on top.
    pub fn get(&self, id: u64) -> Option<WireWindow> {
        let mut w = self.windows.get(&id)?.clone();
        for op in self.ops.iter().filter(|op| op.window == id) {
            match op.kind {
                OpKind::Frame(r) => w.frame = Some(crate::protocol::wire_rect(r)),
                OpKind::Minimize(m) => w.minimized = m,
                OpKind::Raise | OpKind::Focus => {}
            }
        }
        Some(w)
    }

    /// Every known window, frontmost first, with the unconfirmed raises and focuses applied in order.
    pub fn stack(&self) -> Vec<WireWindow> {
        let mut order: Vec<u64> = self.stacking.iter().copied().filter(|id| self.windows.contains_key(id)).collect();
        // A window reported but missing from the stacking list (a delta raced it) goes at the back.
        let mut rest: Vec<u64> = self.windows.keys().copied().filter(|id| !order.contains(id)).collect();
        rest.sort_unstable();
        order.extend(rest);
        for op in &self.ops {
            if matches!(op.kind, OpKind::Raise | OpKind::Focus) {
                if let Some(i) = order.iter().position(|w| *w == op.window) {
                    let id = order.remove(i);
                    order.insert(0, id);
                }
            }
        }
        order.into_iter().filter_map(|id| self.get(id)).collect()
    }

    /// The focused window as the daemon should see it now.
    pub fn focused(&self) -> Option<u64> {
        self.ops.iter().rev().find(|op| op.kind == OpKind::Focus).map(|op| op.window).or(self.focus)
    }

    /// The desktop-file id of the app named `app`, from its installed apps or its windows.
    pub fn app_id(&self, app: &str) -> Option<String> {
        self.apps.iter().find(|a| a.app == app).map(|a| a.app_id.clone()).or_else(|| self.windows.values().find(|w| w.app == app && !w.app_id.is_empty()).map(|w| w.app_id.clone()))
    }

    /// How many ops are outstanding, for tests.
    pub fn pending(&self) -> usize {
        self.ops.len()
    }

    /// Displays in the shape vase-core's `Backend` reports them.
    pub fn core_displays(&self) -> Vec<vase_core::backend::Display> {
        self.displays.iter().map(|d| vase_core::backend::Display { id: d.id, bounds: rect(d.bounds), work_area: rect(d.work_area) }).collect()
    }
}

#[cfg(test)]
#[path = "../tests/unit/mirror.rs"]
mod tests;
