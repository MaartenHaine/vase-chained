//! A fake platform for driving a whole `Daemon` in tests: the displays and windows a test sets up,
//! and a record of what the daemon asked the OS and the painter to do.
//!
//! The backend and the painter share one `World`, which the test keeps a handle to, because the
//! daemon owns both once it is built.

use std::cell::RefCell;
use std::rc::Rc;

use crate::backend::{manageable, Backend, Display, WindowInfo};
use crate::chrome::bar::{Bar, Hits};
use crate::chrome::help::HelpLayout;
use crate::chrome::{usable, BarHits, ListAt, Painter, Position, SwitchRow};
use crate::daemon::{Daemon, Paths};
use crate::geometry::{screen_of, Rect};
use crate::model::Model;
use crate::registry::Registry;
use crate::tree::WindowId;

/// What the fake OS holds, and what the daemon did to it.
#[derive(Default)]
pub struct World {
    pub displays: Vec<Display>,
    /// Frontmost first, as `Backend::list_windows` promises.
    pub windows: Vec<WindowInfo>,
    /// Every `set_frame`, in order.
    pub frames: Vec<(WindowId, Rect)>,
    /// The chrome as last drawn: the tab bar's rect while it is shown.
    pub bar: Option<Rect>,
    /// Whether the tab bar shows the prefix dot.
    pub bar_armed: bool,
    /// Whether a list (the switcher or the picker) is shown.
    pub list: bool,
    /// Whether the shortcut sheet is shown.
    pub help: bool,
}

pub type Shared = Rc<RefCell<World>>;

pub struct FakeBackend(pub Shared);

pub struct FakePainter(pub Shared);

pub type TestDaemon = Daemon<FakeBackend, FakePainter>;

/// One display with no OS furniture: its work area is its bounds.
pub fn display(id: u32, x: f64, y: f64, w: f64, h: f64) -> Display {
    let bounds = Rect::new(x, y, w, h);
    Display { id, bounds, work_area: bounds }
}

/// A normal, tileable window.
pub fn window(id: u64, app: &str, frame: Rect) -> WindowInfo {
    WindowInfo { id: WindowId(id), pid: id as i32, app: app.into(), title: format!("{app} {id}"), frame, layer: 0 }
}

pub fn world(displays: Vec<Display>, windows: Vec<WindowInfo>) -> Shared {
    Rc::new(RefCell::new(World { displays, windows, ..World::default() }))
}

/// Build a daemon over the world the way the platform binaries do at startup: the display at the
/// origin is the main one, the bar's strip comes off it, and every manageable window becomes a tab
/// on the screen its frame sits on. Nothing is persisted.
pub fn daemon(world: &Shared) -> TestDaemon {
    let backend = FakeBackend(Rc::clone(world));
    let displays = backend.displays();
    let screens_cg: Vec<Rect> = displays.iter().map(|d| d.bounds).collect();
    let main_screen = screens_cg.iter().position(|r| r.x == 0.0 && r.y == 0.0).unwrap_or(0);
    let rects: Vec<Rect> = displays.iter().enumerate().map(|(i, d)| usable(d.work_area, i == main_screen, Position::Bottom)).collect();
    let mut registry = Registry::default();
    let mut pairs = Vec::new();
    for w in world.borrow().windows.iter().filter(|w| manageable(w)) {
        registry.adopt(w, false);
        pairs.push((w.id, screen_of(w.frame, &screens_cg)));
    }
    let model = Model::adopt(&rects, &pairs);
    let ids = displays.iter().map(|d| d.id).collect();
    Daemon::new(model, backend, FakePainter(Rc::clone(world)), registry, Paths::default(), main_screen, screens_cg, ids)
}

impl Backend for FakeBackend {
    fn displays(&self) -> Vec<Display> {
        self.0.borrow().displays.clone()
    }

    fn list_windows(&mut self) -> Vec<WindowInfo> {
        self.0.borrow().windows.clone()
    }

    fn all_windows(&mut self) -> Vec<WindowInfo> {
        self.0.borrow().windows.clone()
    }

    fn set_frame(&mut self, window: WindowId, frame: Rect) {
        let mut world = self.0.borrow_mut();
        world.frames.push((window, frame));
        if let Some(w) = world.windows.iter_mut().find(|w| w.id == window) {
            w.frame = frame;
        }
    }

    fn focus(&mut self, window: WindowId) {
        self.raise(window);
    }

    fn raise(&mut self, window: WindowId) {
        let mut world = self.0.borrow_mut();
        if let Some(i) = world.windows.iter().position(|w| w.id == window) {
            let w = world.windows.remove(i);
            world.windows.insert(0, w);
        }
    }

    fn forget(&mut self, _window: WindowId) {}

    fn title(&mut self, window: WindowId) -> Option<String> {
        self.0.borrow().windows.iter().find(|w| w.id == window).map(|w| w.title.clone())
    }

    fn tileable(&mut self, _info: &WindowInfo) -> Option<bool> {
        Some(true)
    }

    fn minimized(&mut self, _window: WindowId) -> Option<bool> {
        Some(false)
    }

    fn minimized_info(&mut self, _info: &WindowInfo) -> Option<bool> {
        Some(false)
    }

    fn set_minimized(&mut self, _window: WindowId, _minimized: bool) {}

    fn fullscreen(&mut self, _info: &WindowInfo) -> Option<bool> {
        Some(false)
    }

    fn close(&mut self, window: WindowId) {
        self.0.borrow_mut().windows.retain(|w| w.id != window);
    }

    fn launchable_apps(&self) -> Vec<String> {
        Vec::new()
    }

    fn launch(&self, _app: &str) {}

    fn warp_cursor(&self, _x: f64, _y: f64) {}
}

impl Painter for FakePainter {
    /// A fixed advance per character: enough for layout to run, and stable across hosts.
    fn measure(&self, text: &str, size: f64) -> f64 {
        text.chars().count() as f64 * size * 0.6
    }

    fn bar(&mut self, bar: &Bar) -> BarHits {
        let mut world = self.0.borrow_mut();
        world.bar = Some(bar.rect);
        world.bar_armed = bar.armed;
        BarHits::default()
    }

    fn prompt(&mut self, rect: Rect, _position: Position, _text: &str) {
        self.0.borrow_mut().bar = Some(rect);
    }

    fn hide_bar(&mut self) {
        self.0.borrow_mut().bar = None;
    }

    fn stack_bars(&mut self, bars: &[Bar]) -> Vec<Hits> {
        bars.iter().map(|_| Hits::new()).collect()
    }

    fn panes(&mut self, _panes: &[(Rect, bool)]) {}

    fn focus_border(&mut self, _rect: Option<Rect>) {}

    fn list(&mut self, _at: ListAt, _header: &str, _rows: &[SwitchRow], _selected: usize) {
        self.0.borrow_mut().list = true;
    }

    fn hide_list(&mut self) {
        self.0.borrow_mut().list = false;
    }

    fn help(&mut self, _layout: &HelpLayout) {
        self.0.borrow_mut().help = true;
    }

    fn hide_help(&mut self) {
        self.0.borrow_mut().help = false;
    }

    fn hide_bars(&mut self) {
        self.0.borrow_mut().bar = None;
    }

    fn hide_all(&mut self) {
        let mut world = self.0.borrow_mut();
        world.bar = None;
        world.list = false;
        world.help = false;
    }

    fn prewarm_icon(&mut self, _app: &str) {}
}
