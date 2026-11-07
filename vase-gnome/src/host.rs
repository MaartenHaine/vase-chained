//! The daemon's half of the protocol, with no I/O: the lifecycle (protocol §7.1), the handshake, and
//! each message turned into calls on vase-core's `Daemon` and back into lines. `server` moves the
//! bytes; everything that decides anything is here, so the L0 tests can drive it whole.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vase_core::backend::{manageable, origin_display, Backend};
use vase_core::config::Config;
use vase_core::daemon::{Daemon, KeyMode, Paths};
use vase_core::geometry::{screen_of, Rect};
use vase_core::input::{Decision, Key, KeyCode, KeyRouter};
use vase_core::model::Effect;
use vase_core::registry::Registry;
use vase_core::state::LiveWindow;

use crate::backend::{GnomeBackend, Shared, SharedRef};
use crate::keys;
use crate::painter::{fit, GnomePainter, GLYPHS, SIZES};
use crate::protocol::{self, error, out, parse, wire_rect, Hello, Incoming, Line, Snapshot, WireKey};

/// The daemon, bound to the GNOME backend and painter.
pub type Vase = Daemon<GnomeBackend, GnomePainter>;

/// How often the platform loop ticks vase (as on Windows: about 25 ms, so a tab number commits on time).
pub const TICK: Duration = Duration::from_millis(25);

/// Reconcile on every this-many ticks: the 100 ms poll.
const RECONCILE_EVERY: u32 = 4;

/// Silence after which the extension counts as gone: twice its 2 s ping, plus slack (§7.1).
pub const SILENCE: Duration = Duration::from_secs(6);

/// What the daemon reports about itself in its hello.
#[derive(Clone, Default)]
pub struct Settings {
    pub paths: Paths,
    /// The build hash of the reviewed SHA, which the extension compares with the installed one.
    pub build: String,
    /// `prctl(PR_GET_MDWE)` at start (S10: `/proc` doesn't show it).
    pub mdwe: bool,
    /// `"full"`, `"partial"` or `"off"`; `None` leaves the field out.
    pub landlock: Option<String>,
}

/// The lifecycle state, as the tests and the log see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    /// No `Daemon` yet: waiting for a hello and a snapshot.
    Starting,
    /// A live extension; ticks, reconcile and effects run.
    Connected(String),
    /// The extension went away: frozen, the model kept and saved.
    Disconnected(String),
}

/// Why the daemon is about to exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// SIGTERM: detached, nothing moved.
    Stop,
    /// `q`, `:quit` or the kill chord: every window put back where it was before vase adopted it.
    Quit,
}

/// What to write, and whether to close the connection afterwards.
#[derive(Debug, Default)]
pub struct Output {
    pub lines: Vec<String>,
    pub close: bool,
}

impl Output {
    fn fatal(lines: Vec<String>) -> Output {
        Output { lines, close: true }
    }
}

/// One connection's state.
struct Conn {
    /// The `seq` of the last line sent.
    seq: u64,
    hello: Option<Hello>,
    snapshot: bool,
    last_rx: Instant,
    mode: Option<Value>,
    hint: Option<Value>,
}

/// A replay the extension must inject after it pops its grab (§6.5).
enum Replay {
    Key(WireKey),
    Click(u32),
}

pub struct Host {
    settings: Settings,
    shared: SharedRef,
    daemon: Option<Vase>,
    router: KeyRouter,
    /// The compositor instance the `Daemon` was built against.
    instance: Option<String>,
    conn: Option<Conn>,
    /// Whether the daemon has let go of the screen since the extension left.
    detached: bool,
    /// The `[[app_focus]]` chords, read with the config the `Daemon` was built from.
    app_focus: Vec<Key>,
    /// Texts asked for with `measure_request` and not answered yet.
    asked: HashSet<(String, u64)>,
    wake: u32,
    exit: Option<Exit>,
}

impl Host {
    pub fn new(settings: Settings) -> Host {
        Host {
            settings,
            shared: Rc::new(RefCell::new(Shared::default())),
            daemon: None,
            router: vase_core::input::router(),
            instance: None,
            conn: None,
            detached: false,
            app_focus: Vec::new(),
            asked: HashSet::new(),
            wake: 0,
            exit: None,
        }
    }

    pub fn lifecycle(&self) -> Lifecycle {
        match (&self.instance, &self.conn) {
            (Some(i), Some(c)) if c.snapshot => Lifecycle::Connected(i.clone()),
            (Some(i), _) if self.daemon.is_some() => Lifecycle::Disconnected(i.clone()),
            _ => Lifecycle::Starting,
        }
    }

    pub fn exit(&self) -> Option<Exit> {
        self.exit
    }

    /// Whether a connection is open (handshake or live).
    pub fn connected(&self) -> bool {
        self.conn.is_some()
    }

    /// The daemon, for the tests.
    pub fn daemon(&self) -> Option<&Vase> {
        self.daemon.as_ref()
    }

    /// The shared mirror and chrome, for the tests.
    pub fn shared(&self) -> SharedRef {
        Rc::clone(&self.shared)
    }

    /// A second connection while one is live is turned away with this (§1).
    pub fn busy_line() -> String {
        error(1, None, "busy", true, "another extension is connected")
    }

    /// The line answering a line over the cap (§2).
    pub fn too_long(&mut self) -> Output {
        let seq = self.next_seq();
        Output::fatal(vec![error(seq, None, "too_long", true, &format!("line over {} bytes", protocol::MAX_LINE))])
    }

    fn next_seq(&mut self) -> u64 {
        match &mut self.conn {
            Some(c) => {
                c.seq += 1;
                c.seq
            }
            None => 0,
        }
    }

    /// A connection was accepted (and passed the peer check): the daemon says hello at once (§4).
    pub fn accept(&mut self, now: Instant) -> Output {
        self.conn = Some(Conn { seq: 0, hello: None, snapshot: false, last_rx: now, mode: None, hint: None });
        self.shared.borrow_mut().surfaces.forget_sent();
        self.asked.clear();
        let mut fields = json!({
            "role": "daemon",
            "build": self.settings.build,
            "pid": std::process::id(),
            "mdwe": self.settings.mdwe,
            "sizes": SIZES,
            "glyphs": GLYPHS,
        });
        if let Some(l) = &self.settings.landlock {
            fields["landlock"] = json!(l);
        }
        let seq = self.next_seq();
        Output { lines: vec![out("hello", seq, fields)], close: false }
    }

    /// The connection is gone (EOF, a fatal error, silence): freeze (§7.1 Disconnected). The model is
    /// kept and saved once, whatever was half-typed is dropped and the chrome hidden; no window moves.
    pub fn eof(&mut self) {
        self.conn = None;
        self.router.disarm();
        if let Some(d) = &mut self.daemon {
            if !self.detached {
                d.detach();
                self.detached = true;
            }
        }
        // Nothing can be sent until a new extension has a snapshot, and that replaces the mirror.
        let mut shared = self.shared.borrow_mut();
        shared.effects.clear();
        shared.mirror.drop_unsent();
    }

    /// SIGTERM (§7.1 Stopping): save and let go, never `restore()`.
    pub fn terminate(&mut self) -> Output {
        self.exit = Some(Exit::Stop);
        if let Some(d) = &mut self.daemon {
            d.detach();
            self.detached = true;
        }
        self.shared.borrow_mut().effects.clear();
        let mut lines = Vec::new();
        if self.conn.as_ref().is_some_and(|c| c.hello.is_some()) {
            let seq = self.next_seq();
            lines.push(out("bye", seq, json!({"reason": "stop"})));
        }
        Output { lines, close: true }
    }

    /// One 25 ms tick. Only a connected daemon ticks: a frozen one runs nothing at all (§7.1).
    pub fn tick(&mut self, now: Instant) -> Output {
        let Some(conn) = &self.conn else { return Output::default() };
        if now.duration_since(conn.last_rx) > SILENCE {
            eprintln!("vase-gnome: no line from the extension for {} s; treating it as gone", SILENCE.as_secs());
            return Output { lines: Vec::new(), close: true };
        }
        if !conn.snapshot {
            return Output::default();
        }
        let Some(d) = &mut self.daemon else { return Output::default() };
        d.tick_tab_entry();
        d.tick_switcher();
        d.tick_pane_picker();
        d.tick_reframe();
        self.wake = self.wake.wrapping_add(1);
        if self.wake.is_multiple_of(RECONCILE_EVERY) {
            d.reconcile();
            d.warm_icons();
        }
        let lines = self.flush(None, Vec::new());
        self.finish(lines)
    }

    /// One line from the extension.
    pub fn receive(&mut self, raw: &[u8], now: Instant) -> Output {
        let Some(conn) = &mut self.conn else { return Output::default() };
        conn.last_rx = now;
        let line = match parse(raw) {
            Ok(line) => line,
            Err(refusal) => {
                eprintln!("vase-gnome: refusing the extension: {}: {}", refusal.code(), refusal.message());
                let seq = self.next_seq();
                return Output::fatal(vec![error(seq, None, refusal.code(), true, &refusal.message())]);
            }
        };
        let Line { seq: re, ack, msg } = line;
        let conn = self.conn.as_mut().unwrap();
        // §10 `order`: hello first, the snapshot before anything that needs the model, and only one of each.
        let order = match (&msg, conn.hello.is_some(), conn.snapshot) {
            (Incoming::Hello(_), false, _) => None,
            (_, false, _) => Some("hello first"),
            (Incoming::Hello(_), true, _) => Some("a second hello"),
            (Incoming::Snapshot(_), true, true) => Some("a second snapshot"),
            // The kill chord works in every state (§8), Handshake included: before a snapshot it quits
            // without the restore's effects, which can't be sent yet (§4).
            (Incoming::Snapshot(_) | Incoming::Measure { .. } | Incoming::Ping | Incoming::Pause | Incoming::Kill | Incoming::Unknown(_), true, _) => None,
            (_, true, false) => Some("the snapshot first"),
            (_, true, true) => None,
        };
        if let Some(why) = order {
            let seq = self.next_seq();
            return Output::fatal(vec![error(seq, Some(re), "order", true, why)]);
        }
        let lines = match msg {
            Incoming::Hello(hello) => return self.hello(hello, re),
            Incoming::Snapshot(s) => return self.snapshot(s, re),
            Incoming::Measure { texts } => {
                let mut answered = false;
                for m in &texts {
                    answered |= self.asked.remove(&(m.text.clone(), m.size.to_bits()));
                }
                for m in texts {
                    self.shared.borrow_mut().surfaces.measurer.add(m);
                }
                if answered {
                    if let (Some(d), true) = (&mut self.daemon, self.conn.as_ref().is_some_and(|c| c.snapshot)) {
                        d.refresh();
                    }
                }
                self.flush(None, Vec::new())
            }
            Incoming::Ping => {
                let seq = self.next_seq();
                vec![out("pong", seq, json!({"re": re}))]
            }
            Incoming::Pause => {
                let seq = self.next_seq();
                vec![error(seq, Some(re), "unsupported", false, "pause is reserved in v1")]
            }
            Incoming::Unknown(t) => {
                let seq = self.next_seq();
                vec![error(seq, Some(re), "unknown_type", false, &format!("unknown t {t:?}"))]
            }
            Incoming::Window(w) => {
                self.shared.borrow_mut().mirror.window(w, ack);
                Vec::new()
            }
            Incoming::WindowGone { id } => {
                self.shared.borrow_mut().mirror.window_gone(id, ack);
                Vec::new()
            }
            Incoming::Stacking { ids } => {
                self.shared.borrow_mut().mirror.stacking(ids, ack);
                Vec::new()
            }
            Incoming::Focus { id } => {
                self.shared.borrow_mut().mirror.focus(id, ack);
                Vec::new()
            }
            Incoming::Displays { displays, primary } => {
                self.shared.borrow_mut().mirror.displays(displays, primary, ack);
                // The screens are rebuilt on the next reconcile; the hint is re-sent then.
                Vec::new()
            }
            Incoming::Settled { id, re: batch, frame } => {
                self.shared.borrow_mut().mirror.settled(id, batch, frame, ack);
                Vec::new()
            }
            Incoming::Key { key, .. } => {
                self.shared.borrow_mut().mirror.ack(ack);
                let replay = self.key(key);
                self.flush(Some(re), replay)
            }
            Incoming::Click { at, button, target } => {
                self.shared.borrow_mut().mirror.ack(ack);
                let replay = self.click(at, button, &target);
                self.flush(Some(re), replay)
            }
            Incoming::End { what, reason } => {
                self.shared.borrow_mut().mirror.ack(ack);
                self.end(&what, &reason);
                self.flush(Some(re), Vec::new())
            }
            Incoming::Kill => {
                self.shared.borrow_mut().mirror.ack(ack);
                if let Some(d) = &mut self.daemon {
                    d.restore();
                }
                self.exit = Some(Exit::Quit);
                self.flush(Some(re), Vec::new())
            }
        };
        self.finish(lines)
    }

    /// After a quit (a command or the kill chord), the restore's effects are followed by `bye`.
    fn finish(&mut self, mut lines: Vec<String>) -> Output {
        let quit = self.exit == Some(Exit::Quit) || self.daemon.as_ref().is_some_and(|d| d.quit_requested());
        if !quit {
            return Output { lines, close: false };
        }
        self.exit = Some(Exit::Quit);
        let seq = self.next_seq();
        lines.push(out("bye", seq, json!({"reason": "quit"})));
        Output { lines, close: true }
    }

    fn hello(&mut self, hello: Hello, re: u64) -> Output {
        if hello.role != "extension" || hello.instance.is_empty() {
            let seq = self.next_seq();
            return Output::fatal(vec![error(seq, Some(re), "bad_json", true, "hello: role must be extension, with an instance")]);
        }
        eprintln!("vase-gnome: extension hello: instance {}, build {:?}, shell {:?}", hello.instance, hello.build, hello.shell);
        self.conn.as_mut().unwrap().hello = Some(hello);
        Output::default()
    }

    /// The snapshot: build the `Daemon` (Starting, or a new compositor instance) or thaw the frozen
    /// one (the same instance), then the first answer (§4, §7.1).
    fn snapshot(&mut self, s: Snapshot, re: u64) -> Output {
        if s.displays.is_empty() || s.windows.iter().any(|w| w.frame.is_none()) {
            let seq = self.next_seq();
            return Output::fatal(vec![error(seq, Some(re), "bad_json", true, "snapshot: needs a display, and a frame for every window")]);
        }
        let instance = self.conn.as_ref().and_then(|c| c.hello.as_ref()).map(|h| h.instance.clone()).unwrap_or_default();
        self.shared.borrow_mut().mirror.load(s.displays, s.primary, s.windows, s.focus, s.apps);
        let same = self.daemon.is_some() && self.instance.as_deref() == Some(instance.as_str());
        if same {
            // Thaw: the mirror now holds the new truth; reconcile forgets only the windows that
            // really closed and adopts the ones that opened, and nothing else moves.
            eprintln!("vase-gnome: the extension is back (instance {instance}); resuming");
            let d = self.daemon.as_mut().unwrap();
            d.refresh();
            d.reconcile();
        } else {
            if let Some(mut old) = self.daemon.take() {
                eprintln!("vase-gnome: a new compositor instance ({instance}); rebuilding the layout from state.json");
                old.detach();
            }
            self.build();
        }
        self.instance = Some(instance);
        self.detached = false;
        self.router.disarm();
        self.conn.as_mut().unwrap().snapshot = true;
        let seq = self.next_seq();
        let mut lines = vec![self.grabs(seq)];
        lines.extend(self.flush(None, Vec::new()));
        self.finish(lines)
    }

    /// Build the `Daemon` the way the other platforms do at startup (`vase-windows/src/bin/vase.rs`):
    /// adopt the manageable windows on screen, then re-match the saved layout onto them, by id
    /// first and then by `(app, title)`, which is what carries a layout across a new shell.
    fn build(&mut self) {
        let paths = self.settings.paths.clone();
        let config = paths.config.as_deref().map(Config::load).unwrap_or_default();
        vase_core::chrome::theme::set_theme(config.theme);
        self.app_focus = config.app_focus.iter().map(|a| a.key).collect();
        let mut backend = GnomeBackend(Rc::clone(&self.shared));
        let bar_position = config.bar_position.unwrap_or(backend.default_bar_position());
        let displays = backend.displays();
        let main_screen = Some(backend.primary_display(&displays)).filter(|&i| i < displays.len()).unwrap_or_else(|| origin_display(&displays));
        let display_ids: Vec<u32> = displays.iter().map(|d| d.id).collect();
        let screens_cg: Vec<Rect> = displays.iter().map(|d| d.bounds).collect();
        let rects: Vec<Rect> = displays.iter().enumerate().map(|(i, d)| vase_core::chrome::usable(d.work_area, i == main_screen, bar_position)).collect();

        let mut windows = Registry::default();
        let mut live = Vec::new();
        for w in backend.list_windows().into_iter().filter(manageable) {
            if backend.tileable(&w) == Some(false) {
                continue;
            }
            windows.adopt(&w, false);
            live.push(LiveWindow { id: w.id, app: w.app.clone(), title: w.title.clone(), screen: screen_of(w.frame, &screens_cg) });
        }
        let saved = paths.state.as_deref().and_then(vase_core::state::load);
        let model = vase_core::state::restore(saved, &live, &rects);
        let painter = GnomePainter(Rc::clone(&self.shared));
        let mut d = Daemon::new(model, backend, painter, windows, paths, main_screen, screens_cg, display_ids);
        let model = d.model.as_ref().unwrap();
        let mut effects = vec![Effect::Render(model.placements())];
        if let Some(w) = model.focused_window() {
            effects.push(Effect::FocusWindow(w));
        }
        d.execute(effects);
        d.warm_window_icons();
        d.refresh();
        eprintln!("vase-gnome: adopted {} windows", live.len());
        self.daemon = Some(d);
    }

    /// The chords the extension grabs in Idle (§6.6).
    fn grabs(&self, seq: u64) -> String {
        let mut chords = Vec::new();
        for (key, role) in [(vase_core::input::prefix(), "prefix"), (vase_core::input::stack_prefix(), "prefix")] {
            if let Some(w) = keys::wire(key) {
                chords.push(json!({"key": w.to_json(), "role": role}));
            }
        }
        for key in &self.app_focus {
            if let Some(w) = keys::wire(*key) {
                chords.push(json!({"key": w.to_json(), "role": "app_focus"}));
            }
        }
        out("grabs", seq, json!({"chords": chords}))
    }

    /// A key the extension took (§5.3), run the way the macOS and Windows hooks run one: the modal
    /// overlays first (`intercept_key`), then the prefix router. Returns the keys to replay.
    fn key(&mut self, wire: WireKey) -> Vec<Replay> {
        let Some(d) = &mut self.daemon else { return Vec::new() };
        // The extension never sends these (S9); if one comes anyway it is nobody's key.
        if keys::is_modifier(wire.code) {
            return Vec::new();
        }
        // A key vase has no name for matches no binding and goes straight back, the router untouched,
        // as on Windows (`vase-windows/src/hook.rs`).
        let Some(key) = keys::key(&wire) else { return vec![Replay::Key(wire)] };
        if d.intercept_key(key) {
            return Vec::new();
        }
        let was = self.router.is_armed();
        let decision = self.router.key(key);
        let now = self.router.is_armed();
        if was != now {
            d.prefix_armed = now;
            d.refresh();
        }
        match decision {
            Decision::PassThrough => vec![Replay::Key(wire)],
            Decision::Consume => Vec::new(),
            Decision::ConsumeAndRun(cmd) => {
                d.run(cmd);
                Vec::new()
            }
        }
    }

    /// A click (§5.4): on the chrome, vase routes it; outside an overlay, the overlay closes and the
    /// click goes back to whatever was under it.
    fn click(&mut self, at: [f64; 2], button: u32, target: &str) -> Vec<Replay> {
        let Some(d) = &mut self.daemon else { return Vec::new() };
        match target {
            "chrome" => {
                d.click(at[0], at[1]);
                Vec::new()
            }
            _ => {
                close_overlay(d);
                vec![Replay::Click(button)]
            }
        }
    }

    /// The extension ended a state on its own: a timeout, a revoked grab, a focus change (§5.5).
    fn end(&mut self, what: &str, reason: &str) {
        let Some(d) = &mut self.daemon else { return };
        match what {
            "prefix" => {
                self.router.disarm();
                d.cancel_prefix();
            }
            "sticky" => {
                d.end_sticky();
            }
            "overlay" => close_overlay(d),
            _ => eprintln!("vase-gnome: end of an unknown state {what:?} ({reason}); ignored"),
        }
    }

    /// Everything a dispatch produced, in the order the extension applies it: the effects batch, the
    /// chrome, measuring requests, replays, the placement hint, and last the key mode (§6.4).
    fn flush(&mut self, re: Option<u64>, replay: Vec<Replay>) -> Vec<String> {
        let mut lines = Vec::new();
        // Before its snapshot the extension gets nothing but answers to pings and errors (§4).
        if !self.conn.as_ref().is_some_and(|c| c.snapshot) {
            return lines;
        }
        let effects = std::mem::take(&mut self.shared.borrow_mut().effects);
        if !effects.is_empty() {
            let seq = self.next_seq();
            self.shared.borrow_mut().mirror.seal(seq);
            let mut fields = json!({"effects": effects});
            if let Some(re) = re {
                fields["re"] = json!(re);
            }
            lines.push(out("effects", seq, fields));
        }
        let draw = self.shared.borrow_mut().surfaces.take();
        if let Some(mut fields) = draw {
            fit(&mut fields);
            let seq = self.next_seq();
            lines.push(out("draw", seq, Value::Object(fields)));
        }
        let requests = self.shared.borrow_mut().surfaces.take_requests();
        if !requests.is_empty() {
            let mut texts: Vec<String> = Vec::new();
            let mut sizes: Vec<f64> = Vec::new();
            for (text, size) in requests {
                self.asked.insert((text.clone(), size.to_bits()));
                if !texts.contains(&text) {
                    texts.push(text);
                }
                if !sizes.contains(&size) {
                    sizes.push(size);
                }
            }
            let seq = self.next_seq();
            lines.push(out("measure_request", seq, json!({"texts": texts, "sizes": sizes})));
        }
        for r in replay {
            let seq = self.next_seq();
            let fields = match r {
                Replay::Key(k) => json!({"re": re, "keys": [k.to_json()]}),
                Replay::Click(button) => json!({"re": re, "click": {"button": button}}),
            };
            lines.push(out("replay", seq, fields));
        }
        if let Some(hint) = self.hint() {
            if self.conn.as_ref().unwrap().hint.as_ref() != Some(&hint) {
                let seq = self.next_seq();
                lines.push(out("placement_hint", seq, hint.clone()));
                self.conn.as_mut().unwrap().hint = Some(hint);
            }
        }
        if let Some(mode) = self.mode() {
            // A key's answer always ends with its mode; otherwise only a change is sent.
            if re.is_some() || self.conn.as_ref().unwrap().mode.as_ref() != Some(&mode) {
                let seq = self.next_seq();
                let mut fields = mode.clone();
                if let Some(re) = re {
                    fields["re"] = json!(re);
                }
                lines.push(out("mode", seq, fields));
                self.conn.as_mut().unwrap().mode = Some(mode);
            }
        }
        lines
    }

    /// The key mode as the `mode` line carries it (§6.4, seam K2).
    fn mode(&self) -> Option<Value> {
        let d = self.daemon.as_ref()?;
        Some(match d.key_mode() {
            KeyMode::Idle => json!({"mode": "idle"}),
            KeyMode::Prefix => json!({"mode": "prefix"}),
            KeyMode::Overlay => {
                let kind = if d.prompt.is_some() {
                    "prompt"
                } else if d.switcher.is_some() {
                    "switcher"
                } else if d.pane_picker.is_some() {
                    "picker"
                } else {
                    "help"
                };
                json!({"mode": "overlay", "kind": kind})
            }
            KeyMode::Sticky(keys) => {
                let keys: Vec<Value> = keys.into_iter().filter_map(keys::wire).map(|k| k.to_json()).collect();
                json!({"mode": "sticky", "keys": keys})
            }
            KeyMode::PendingLaunch => json!({"mode": "pending_launch"}),
        })
    }

    /// Where the next new window goes (§6.3, S6): the focused screen's content rect, which a new tab's
    /// single pane fills, or for a pending launch's app the empty pane it was launched into.
    fn hint(&self) -> Option<Value> {
        let d = self.daemon.as_ref()?;
        let model = d.model.as_ref()?;
        let default = model.screens.get(model.focused_screen).map(|s| wire_rect(s.rect));
        let mut apps = serde_json::Map::new();
        if let (Some(p), Some(rect)) = (&d.pending_launch, model.focused_pane_rect()) {
            if let Some(app_id) = self.shared.borrow().mirror.app_id(&p.app) {
                apps.insert(app_id, json!(wire_rect(rect)));
            }
        }
        Some(json!({"default": default, "apps": apps}))
    }
}

/// Close whatever overlay is open the way Esc would. The pane picker is left alone: Esc there closes
/// its pane, and what a dismissed picker should do is open (protocol §12.2, M1c).
fn close_overlay(d: &mut Vase) {
    let picker_only = d.pane_picker.is_some() && d.prompt.is_none() && d.switcher.is_none();
    if d.key_mode() == KeyMode::Overlay && !picker_only {
        d.intercept_key(Key::plain(KeyCode::Escape));
    }
}

#[cfg(test)]
#[path = "../tests/unit/host.rs"]
mod tests;
