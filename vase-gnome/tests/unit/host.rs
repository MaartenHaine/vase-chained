//! The host, driven line by line with no socket: what each message makes vase do, and what goes back
//! (protocol §4-§8). The socket half is in `tests/l0.rs`.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;

/// The extension's side of one connection, as these tests play it.
struct Ext {
    host: Host,
    seq: u64,
    /// The newest effects batch "applied": every batch is applied as soon as it arrives.
    ack: u64,
    now: Instant,
}

/// The owner's layout (F11): a 2560x1440 monitor at the origin and the primary 1920x1080 at x=2560,
/// with GNOME's top panel (32 px) on the primary only.
fn displays() -> Value {
    json!([
        {"id": 1, "connector": "DP-1", "bounds": [0, 0, 2560, 1440], "work_area": [0, 0, 2560, 1440], "scale": 1.5},
        {"id": 2, "connector": "HDMI-1", "bounds": [2560, 0, 1920, 1080], "work_area": [2560, 32, 1920, 1048], "scale": 1.0}
    ])
}

fn window(id: u64, app: &str, frame: [f64; 4]) -> Value {
    json!({"id": id, "pid": id, "app": app, "app_id": format!("org.example.{app}.desktop"), "title": format!("{app} {id}"), "frame": frame})
}

/// Two windows on the left monitor, which vase focuses first, and one on the primary.
fn windows() -> Value {
    json!([window(10, "Ghostty", [100.0, 100.0, 800.0, 600.0]), window(11, "Files", [900.0, 100.0, 800.0, 600.0]), window(12, "Calculator", [2700.0, 100.0, 400.0, 500.0])])
}

fn key(code: u32, mods: &[&str]) -> Value {
    json!({"key": {"code": code, "mods": mods}, "time_us": 1, "via": "grab"})
}

const ALT_A: u32 = 30;
const BACKSLASH: u32 = 43;

fn settings(state: Option<PathBuf>) -> Settings {
    Settings { paths: Paths { config: None, state }, build: "test".into(), mdwe: false, landlock: None }
}

impl Ext {
    fn new(state: Option<PathBuf>) -> Ext {
        Ext { host: Host::new(settings(state)), seq: 0, ack: 0, now: Instant::now() }
    }

    fn read(&mut self, out: Output) -> Vec<Value> {
        let lines: Vec<Value> = out.lines.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
        for l in &lines {
            assert_eq!(l["v"], 1);
            if l["t"] == "effects" {
                self.ack = l["seq"].as_u64().unwrap();
            }
        }
        lines
    }

    fn send(&mut self, t: &str, mut fields: Value) -> Vec<Value> {
        self.seq += 1;
        fields["v"] = json!(1);
        fields["t"] = json!(t);
        fields["seq"] = json!(self.seq);
        fields["ack"] = json!(self.ack);
        let out = self.host.receive(fields.to_string().as_bytes(), self.now);
        self.read(out)
    }

    /// Connect, hello and snapshot: the daemon's whole first answer, its hello first.
    fn connect(&mut self, instance: &str, windows: Value) -> Vec<Value> {
        self.seq = 0;
        self.ack = 0;
        let out = self.host.accept(self.now);
        let mut lines = self.read(out);
        lines.extend(self.send("hello", json!({"role": "extension", "instance": instance, "build": "x", "shell": "50.1"})));
        // What the extension measures before its snapshot (§4): the glyph table, then every title.
        let mut texts: Vec<String> = crate::painter::GLYPHS.chars().map(String::from).collect();
        texts.extend(windows.as_array().unwrap().iter().map(|w| w["title"].as_str().unwrap().to_string()));
        lines.extend(self.measure(&texts));
        let apps = json!([{"app": "Text Editor", "app_id": "org.gnome.TextEditor.desktop"}]);
        lines.extend(self.send("snapshot", json!({"displays": displays(), "primary": 1, "windows": windows, "focus": 10, "apps": apps})));
        lines
    }

    /// Pango, as these tests fake it: 7 px per character at every size.
    fn measure(&mut self, texts: &[String]) -> Vec<Value> {
        let mut entries = Vec::new();
        for size in crate::painter::SIZES {
            for t in texts {
                let advances: Vec<f64> = (1..=t.chars().count()).map(|i| i as f64 * 7.0).collect();
                entries.push(json!({"text": t, "size": size, "advances": advances}));
            }
        }
        self.send("measure", json!({"texts": entries}))
    }

    fn key(&mut self, code: u32, mods: &[&str]) -> Vec<Value> {
        self.send("key", key(code, mods))
    }

    /// Run the 25 ms ticks for `n` ticks (n/4 reconciles).
    fn ticks(&mut self, n: u32) -> Vec<Value> {
        let mut lines = Vec::new();
        for _ in 0..n {
            self.now += TICK;
            let out = self.host.tick(self.now);
            lines.extend(self.read(out));
        }
        lines
    }
}

fn of<'a>(lines: &'a [Value], t: &str) -> Vec<&'a Value> {
    lines.iter().filter(|l| l["t"] == t).collect()
}

fn places(lines: &[Value]) -> Vec<(u64, Vec<f64>)> {
    of(lines, "effects")
        .iter()
        .flat_map(|l| l["effects"].as_array().unwrap().iter())
        .filter(|e| e["k"] == "place")
        .map(|e| (e["id"].as_u64().unwrap(), e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()))
        .collect()
}

fn last_mode(lines: &[Value]) -> Value {
    of(lines, "mode").last().map(|m| (*m).clone()).expect("a mode line")
}

#[test]
fn the_handshake_builds_the_daemon_and_answers_with_grabs_effects_chrome_hint_and_mode() {
    let mut ext = Ext::new(None);
    let lines = ext.connect("boot:100:1", windows());
    let types: Vec<&str> = lines.iter().map(|l| l["t"].as_str().unwrap()).collect();
    assert_eq!(types.first(), Some(&"hello"));
    assert_eq!(lines[0]["role"], "daemon");
    assert_eq!(lines[0]["sizes"], json!([12.0, 13.0]));
    assert_eq!(types.last(), Some(&"mode"), "the mode is the last line of an answer: {types:?}");
    for t in ["grabs", "effects", "draw", "placement_hint"] {
        assert!(types.contains(&t), "{t} missing from {types:?}");
    }
    let grabs = &of(&lines, "grabs")[0]["chords"];
    assert_eq!(grabs[0], json!({"key": {"code": ALT_A, "mods": ["alt"]}, "role": "prefix"}));
    assert_eq!(grabs[1], json!({"key": {"code": 18, "mods": ["alt"]}, "role": "prefix"}));
    assert_eq!(last_mode(&lines)["mode"], "idle");
    assert_eq!(ext.host.lifecycle(), Lifecycle::Connected("boot:100:1".into()));
    // Every window got a tab; the current tab of each screen is placed on its content rect, and the
    // Files tab behind the Ghostty one is left where it is (hidden tabs are covered, not moved).
    let placed: Vec<u64> = places(&lines).iter().map(|(id, _)| *id).collect();
    assert_eq!(placed, vec![10, 12]);
}

#[test]
fn the_primary_at_x2560_gets_the_tab_bar_and_its_strip() {
    let mut ext = Ext::new(None);
    let lines = ext.connect("i", windows());
    let draw = of(&lines, "draw");
    let bar = &draw.last().unwrap()["bar"];
    let strip = vase_core::chrome::bar_height();
    assert_eq!(bar["rect"], json!([2560.0, 32.0, 1920.0, strip]), "the bar sits at the top of the primary's work area");
    // The Calculator, on the primary, fills the primary's work area below the strip.
    assert!(places(&lines).contains(&(12, vec![2560.0, 32.0 + strip, 1920.0, 1048.0 - strip])));
}

#[test]
fn a_prefix_command_produces_its_effects_and_key_modes() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    let armed = ext.key(ALT_A, &["alt"]);
    assert_eq!(last_mode(&armed)["mode"], "prefix");
    assert_eq!(last_mode(&armed)["re"], ext.seq);
    assert!(of(&armed, "effects").is_empty());
    let split = ext.key(BACKSLASH, &[]);
    // The focused window (the first tab on the left monitor) takes the left half; the new empty pane
    // on the right gets the picker.
    assert!(places(&split).contains(&(10, vec![0.0, 0.0, 1280.0, 1440.0])), "{:?}", places(&split));
    let mode = last_mode(&split);
    assert_eq!(mode["mode"], "overlay");
    assert_eq!(mode["kind"], "picker");
    let list = &of(&split, "draw").last().unwrap()["list"];
    assert_eq!(list["rect"], json!([1280.0, 0.0, 1280.0, 1440.0]));
    assert_eq!(list["centered"], false);
    let types: Vec<&str> = split.iter().map(|l| l["t"].as_str().unwrap()).collect();
    assert_eq!(types.last(), Some(&"mode"), "{types:?}");
}

#[test]
fn an_unbound_key_and_a_second_prefix_are_replayed_before_the_mode_that_ends_the_prefix() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    ext.key(ALT_A, &["alt"]);
    let lines = ext.key(21 /* y: no row after the prefix */, &[]);
    let types: Vec<&str> = lines.iter().map(|l| l["t"].as_str().unwrap()).collect();
    assert_eq!(types, vec!["draw", "replay", "mode"], "the prefix dot clears, then the replay, then the mode");
    assert_eq!(of(&lines, "replay")[0]["keys"], json!([{"code": 21, "mods": []}]));
    assert_eq!(last_mode(&lines)["mode"], "idle");

    // "Alt+A, then Alt+A": the second has no row, so it goes back to the app as one Alt+A.
    ext.key(ALT_A, &["alt"]);
    let lines = ext.key(ALT_A, &["alt"]);
    assert_eq!(of(&lines, "replay")[0]["keys"], json!([{"code": ALT_A, "mods": ["alt"]}]));
    assert_eq!(last_mode(&lines)["mode"], "idle");
    let r = lines.iter().position(|l| l["t"] == "replay").unwrap();
    let m = lines.iter().position(|l| l["t"] == "mode").unwrap();
    assert!(r < m);
    // Esc after a prefix is no different: no row, replayed.
    ext.key(ALT_A, &["alt"]);
    let lines = ext.key(1, &[]);
    assert_eq!(of(&lines, "replay")[0]["keys"], json!([{"code": 1, "mods": []}]));
}

#[test]
fn the_prefix_timeout_disarms_the_router() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    ext.key(ALT_A, &["alt"]);
    let lines = ext.send("end", json!({"what": "prefix", "reason": "timeout"}));
    assert_eq!(last_mode(&lines)["mode"], "idle");
    // The next `\` is a plain key again, not the prefix's split.
    let lines = ext.key(BACKSLASH, &[]);
    assert!(places(&lines).is_empty());
    assert_eq!(of(&lines, "replay").len(), 1);
}

#[test]
fn a_resize_goes_sticky_and_its_timer_ends_it() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    ext.key(ALT_A, &["alt"]);
    ext.key(BACKSLASH, &[]);
    // Fill the new pane from the picker (Down, Return: its second row, Files), so the tab has two windows.
    ext.key(108, &[]);
    ext.key(28, &[]);
    ext.key(ALT_A, &["alt"]);
    let lines = ext.key(105 /* Left */, &["shift"]);
    let mode = last_mode(&lines);
    assert_eq!(mode["mode"], "sticky", "{mode}");
    let keys = mode["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 8);
    assert!(keys.contains(&json!({"code": 105, "mods": ["shift"]})));
    assert!(keys.contains(&json!({"code": 35, "mods": ["shift"]})));
    // A repeat key resizes again without a prefix.
    let lines = ext.send("key", json!({"key": {"code": 105, "mods": ["shift"]}, "via": "temporary"}));
    assert!(!places(&lines).is_empty());
    assert_eq!(last_mode(&lines)["mode"], "sticky");
    let lines = ext.send("end", json!({"what": "sticky", "reason": "timeout"}));
    assert_eq!(last_mode(&lines)["mode"], "idle");
}

#[test]
fn a_pending_launch_takes_only_esc_and_the_hint_points_at_its_pane() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    ext.key(ALT_A, &["alt"]);
    ext.key(BACKSLASH, &[]);
    // Search the picker for the Text Editor's launch row ("/edit", Return) and pick it.
    let mut lines = Vec::new();
    for code in [53, 18, 32, 23, 20, 28] {
        lines = ext.key(code, &[]);
    }
    let launch: Vec<&Value> = of(&lines, "effects").iter().flat_map(|l| l["effects"].as_array().unwrap().iter()).filter(|e| e["k"] == "launch").collect();
    assert_eq!(launch, vec![&json!({"k": "launch", "app_id": "org.gnome.TextEditor.desktop"})]);
    assert_eq!(last_mode(&lines)["mode"], "pending_launch");
    let hint = of(&lines, "placement_hint").last().map(|h| (*h).clone()).unwrap();
    assert_eq!(hint["apps"]["org.gnome.TextEditor.desktop"], json!([1280.0, 0.0, 1280.0, 1440.0]));
    // Esc cancels the launch and closes the pane.
    let lines = ext.key(1, &[]);
    assert_eq!(last_mode(&lines)["mode"], "idle");
    assert!(of(&lines, "replay").is_empty(), "Esc was the launch's");
}

#[test]
fn the_placement_hint_follows_focus() {
    let mut ext = Ext::new(None);
    let lines = ext.connect("i", windows());
    let hint = of(&lines, "placement_hint")[0].clone();
    assert_eq!(hint["default"], json!([0.0, 0.0, 2560.0, 1440.0]));
    // The user clicks the Calculator, on the primary.
    ext.send("stacking", json!({"ids": [12, 10, 11]}));
    ext.send("focus", json!({"id": 12}));
    let lines = ext.ticks(8);
    let hint = of(&lines, "placement_hint").last().map(|h| (*h).clone()).expect("a new hint");
    let strip = vase_core::chrome::bar_height();
    assert_eq!(hint["default"], json!([2560.0, 32.0 + strip, 1920.0, 1048.0 - strip]));
}

#[test]
fn a_new_window_from_a_delta_becomes_a_tab() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    ext.send("measure", json!({"texts": [{"text": "Text 13", "size": 12.0, "advances": [7, 14, 21, 28, 31, 38, 45]}]}));
    ext.send("window", window(13, "Text", [10.0, 10.0, 500.0, 500.0]));
    let lines = ext.ticks(8);
    assert!(places(&lines).iter().any(|(id, _)| *id == 13), "{lines:?}");
    // And a gone one is forgotten.
    ext.send("window_gone", json!({"id": 13}));
    ext.ticks(8);
    let model = ext.host.daemon().unwrap().model.as_ref().unwrap();
    assert!(!model.all_windows().contains(&vase_core::tree::WindowId(13)));
}

#[test]
fn a_disconnected_daemon_runs_nothing_and_resumes_without_moving_a_window() {
    let dir = tempdir("disc");
    let state = dir.join("state.json");
    let mut ext = Ext::new(Some(state.clone()));
    ext.connect("boot:1:1", windows());
    ext.key(ALT_A, &["alt"]);
    ext.host.eof();
    assert_eq!(ext.host.lifecycle(), Lifecycle::Disconnected("boot:1:1".into()));
    assert!(state.exists(), "the layout is saved when the extension goes");
    for _ in 0..200 {
        ext.now += TICK;
        assert!(ext.host.tick(ext.now).lines.is_empty());
    }
    assert!(ext.host.shared().borrow().effects.is_empty());
    assert_eq!(ext.host.daemon().unwrap().key_mode(), KeyMode::Idle, "the half-typed prefix went with the keyboard");

    // The same shell comes back (an unlock); one window closed meanwhile, the others are where vase put them.
    let frames = |id: u64| match id {
        10 => [0.0, 0.0, 2560.0, 1440.0],
        11 => [0.0, 0.0, 2560.0, 1440.0],
        _ => [2560.0, 32.0 + vase_core::chrome::bar_height(), 1920.0, 1048.0 - vase_core::chrome::bar_height()],
    };
    let back = json!([window(12, "Calculator", frames(12)), window(10, "Ghostty", frames(10))]);
    let lines = ext.connect("boot:1:1", back);
    assert_eq!(ext.host.lifecycle(), Lifecycle::Connected("boot:1:1".into()));
    assert!(places(&lines).is_empty(), "nothing moves on a resume: {:?}", places(&lines));
    let model = ext.host.daemon().unwrap().model.as_ref().unwrap();
    let mut ids: Vec<u64> = model.all_windows().iter().map(|w| w.0).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![10, 12], "only the window that really closed is forgotten");
}

#[test]
fn a_new_compositor_instance_rematches_the_layout_by_app_and_title() {
    let dir = tempdir("inst");
    let state = dir.join("state.json");
    let mut ext = Ext::new(Some(state.clone()));
    ext.connect("boot:1:1", windows());
    ext.key(ALT_A, &["alt"]);
    ext.key(BACKSLASH, &[]);
    ext.key(108, &[]); // the picker's second row (the first is the Ghostty beside the pane) ...
    ext.key(28, &[]); // ... Files, moves into the new pane
    let before = ext.host.daemon().unwrap().model.as_ref().unwrap().placements();
    ext.host.eof();
    // A re-login: new window ids (F23), the same apps and titles.
    let renamed = |w: Value, id: u64| {
        let mut w = w;
        w["id"] = json!(id);
        w
    };
    let windows = json!([
        renamed(window(10, "Ghostty", [0.0, 0.0, 300.0, 300.0]), 910),
        renamed(window(11, "Files", [0.0, 0.0, 300.0, 300.0]), 911),
        renamed(window(12, "Calculator", [2600.0, 50.0, 300.0, 300.0]), 912)
    ]);
    ext.connect("boot:1:2", windows);
    let after = ext.host.daemon().unwrap().model.as_ref().unwrap().placements();
    let map = |id: u64| id + 900;
    let mut expected: Vec<(u64, Rect)> = before.iter().map(|(w, r)| (map(w.0), *r)).collect();
    let mut got: Vec<(u64, Rect)> = after.iter().map(|(w, r)| (w.0, *r)).collect();
    expected.sort_by_key(|(id, _)| *id);
    got.sort_by_key(|(id, _)| *id);
    assert_eq!(got, expected);
}

#[test]
fn sigterm_saves_and_lets_go_without_a_placement() {
    let dir = tempdir("term");
    let state = dir.join("state.json");
    let mut ext = Ext::new(Some(state.clone()));
    ext.connect("i", windows());
    let out = ext.host.terminate();
    let lines = ext.read(out);
    assert!(places(&lines).is_empty());
    assert_eq!(of(&lines, "bye")[0]["reason"], "stop");
    assert!(state.exists());
    assert_eq!(ext.host.exit(), Some(Exit::Stop));
}

#[test]
fn the_kill_chord_restores_every_window_then_says_bye() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    let out = ext.send("kill", json!({}));
    let placed = places(&out);
    assert!(placed.contains(&(10, vec![100.0, 100.0, 800.0, 600.0])), "{placed:?}");
    assert_eq!(out.last().unwrap()["t"], "bye");
    assert_eq!(out.last().unwrap()["reason"], "quit");
    assert_eq!(ext.host.exit(), Some(Exit::Quit));
}

/// Accept, hello, then the kill chord with no snapshot: the whole answer to the kill.
fn kill_in_handshake(ext: &mut Ext) -> Output {
    ext.seq = 0;
    ext.ack = 0;
    let out = ext.host.accept(ext.now);
    ext.read(out);
    ext.send("hello", json!({"role": "extension", "instance": "i", "build": "x", "shell": "50.1"}));
    ext.seq += 1;
    ext.host.receive(json!({"v": 1, "t": "kill", "seq": ext.seq, "ack": 0}).to_string().as_bytes(), ext.now)
}

#[test]
fn the_kill_chord_before_the_snapshot_quits_with_only_a_bye() {
    // Starting: no daemon yet.
    let mut ext = Ext::new(None);
    let out = kill_in_handshake(&mut ext);
    assert!(out.close);
    let lines = ext.read(out);
    assert_eq!(lines.iter().map(|l| l["t"].as_str().unwrap()).collect::<Vec<_>>(), vec!["bye"], "{lines:?}");
    assert_eq!(lines[0]["reason"], "quit");
    assert_eq!(ext.host.exit(), Some(Exit::Quit));

    // Disconnected: the frozen daemon restores and quits, but no effect goes out before a snapshot (§4).
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    ext.host.eof();
    let out = kill_in_handshake(&mut ext);
    assert!(out.close);
    let lines = ext.read(out);
    assert_eq!(lines.iter().map(|l| l["t"].as_str().unwrap()).collect::<Vec<_>>(), vec!["bye"], "{lines:?}");
    assert_eq!(lines[0]["reason"], "quit");
    assert_eq!(ext.host.exit(), Some(Exit::Quit));
}

#[test]
fn lines_out_of_order_are_refused() {
    let mut ext = Ext::new(None);
    let out = ext.host.accept(ext.now);
    ext.read(out);
    let lines = ext.key(ALT_A, &["alt"]);
    assert_eq!(lines[0]["code"], "order");
    assert_eq!(lines[0]["fatal"], true);

    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    let lines = ext.send("snapshot", json!({"displays": displays(), "windows": []}));
    assert_eq!(lines[0]["code"], "order");
}

#[test]
fn a_v2_line_an_unknown_type_and_pause_get_their_errors() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    let lines = ext.send("telepathy", json!({}));
    assert_eq!((lines[0]["code"].as_str(), lines[0]["fatal"].as_bool()), (Some("unknown_type"), Some(false)));
    let lines = ext.send("pause", json!({"paused": true}));
    assert_eq!(lines[0]["code"], "unsupported");
    let out = ext.host.receive(br#"{"v": 2, "t": "ping", "seq": 99}"#, ext.now);
    assert!(out.close);
    let lines = ext.read(out);
    assert_eq!(lines[0]["code"], "version");
    let out = ext.host.receive(b"[1, 2]", ext.now);
    assert!(out.close);
    assert_eq!(ext.read(out)[0]["code"], "bad_json");
}

#[test]
fn a_silent_extension_counts_as_gone() {
    let mut ext = Ext::new(None);
    ext.connect("i", windows());
    ext.now += SILENCE + Duration::from_millis(1);
    assert!(ext.host.tick(ext.now).close);
    ext.now = Instant::now();
    let lines = ext.send("ping", json!({}));
    assert_eq!(lines[0]["t"], "pong");
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vg-host-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
