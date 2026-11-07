//! L0 (wayfinder §5): the real `vase-gnome` binary over its real socket, driven by the fake
//! extension in `fake/`. What the daemon decides line by line is in `tests/unit/host.rs`; these are
//! the parts only a process and a socket can show: the handshake on the wire, the refusals, the line
//! cap, the peer check, the lock, freezing and thawing across connections, and SIGTERM.
#![cfg(target_os = "linux")]

mod fake;

use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use fake::{connect, of, places, tempdir, window, Daemon, Fake};
use serde_json::{json, Value};

const ALT_A: u32 = 30;
const BACKSLASH: u32 = 43;

/// The content rect of the one test monitor: its work area less the bar's strip at the top.
fn content() -> [f64; 4] {
    let strip = 28.0; // the default (native) style's bar height
    [0.0, 32.0 + strip, 1920.0, 1048.0 - strip]
}

fn two_windows() -> Value {
    json!([window(10, "Ghostty", [100.0, 100.0, 800.0, 600.0]), window(11, "Files", [900.0, 100.0, 800.0, 600.0])])
}

#[test]
fn handshake_then_a_prefix_command_over_the_socket() {
    let dir = tempdir("l0-hand");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    let lines = ext.handshake("boot:1:1", two_windows());
    let hello = &lines[0];
    assert_eq!(hello["pid"], d.child.id());
    assert!(hello["mdwe"].is_boolean());
    // The two prefixes, then the default config's `[[app_focus]]` chords.
    let chords = of(&lines, "grabs")[0]["chords"].as_array().unwrap().clone();
    assert_eq!(chords[0]["role"], "prefix");
    assert_eq!(chords[1]["role"], "prefix");
    assert!(chords[2..].iter().all(|c| c["role"] == "app_focus"), "{chords:?}");
    assert_eq!(places(&lines), vec![(10, content())], "the current tab is placed; the Files tab behind it is not moved");
    assert_eq!(lines.last().unwrap()["mode"], "idle");

    assert_eq!(ext.key(ALT_A, &["alt"]).last().unwrap()["mode"], "prefix");
    let lines = ext.key(BACKSLASH, &[]);
    let c = content();
    assert_eq!(places(&lines), vec![(10, [c[0], c[1], c[2] / 2.0, c[3]])]);
    let mode = lines.last().unwrap();
    assert_eq!((mode["mode"].as_str(), mode["kind"].as_str()), (Some("overlay"), Some("picker")));
    assert!(of(&lines, "draw").iter().any(|d| d["list"]["rect"] == json!([c[2] / 2.0, c[1], c[2] / 2.0, c[3]])));
}

#[test]
fn a_delta_after_the_snapshot_adopts_a_new_window() {
    let dir = tempdir("l0-delta");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
    ext.send("measure", json!({"texts": [{"text": "Text 12", "size": 12.0, "advances": fake::advances("Text 12")}]}));
    ext.send("window", window(12, "Text", [300.0, 300.0, 500.0, 400.0]));
    ext.send("stacking", json!({"ids": [12, 10, 11]}));
    let lines = ext.until("effects");
    assert_eq!(places(&lines), vec![(12, content())], "a new window opens as a tab of its own, in front");
    ext.send("window_gone", json!({"id": 12}));
    let lines = ext.drain(Duration::from_millis(400));
    assert!(places(&lines).iter().all(|(id, _)| *id != 12), "a gone window is never placed: {lines:?}");
}

#[test]
fn a_v2_line_is_refused_and_the_connection_closed() {
    let dir = tempdir("l0-v2");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.until("hello");
    ext.send_raw(b"{\"v\": 2, \"t\": \"hello\", \"seq\": 1, \"role\": \"extension\", \"instance\": \"x\"}\n");
    let lines = ext.until_eof();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!((lines[0]["code"].as_str(), lines[0]["fatal"].as_bool()), (Some("version"), Some(true)));
    // The daemon is still there for the next (right) version.
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
}

#[test]
fn a_line_over_one_mib_is_refused_without_being_read_whole() {
    let dir = tempdir("l0-cap");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.until("hello");
    let mut writer = ext.stream.try_clone().unwrap();
    writer.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let flood = std::thread::spawn(move || {
        let chunk = vec![b'x'; 64 * 1024];
        let mut sent = 0usize;
        while sent < 8 << 20 {
            match writer.write(&chunk) {
                Ok(n) => sent += n,
                Err(_) => break,
            }
        }
        sent
    });
    let lines = ext.until_eof();
    assert_eq!(lines.last().unwrap()["code"], "too_long", "{lines:?}");
    let sent = flood.join().unwrap();
    assert!(sent < 4 << 20, "the daemon read {sent} bytes of an 8 MiB line before refusing it");
}

#[test]
fn the_confined_daemon_reports_landlock_in_its_hello() {
    let dir = tempdir("l0-ll");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    let hello = ext.until("hello");
    let landlock = hello[0]["landlock"].as_str().unwrap().to_string();
    assert!(["full", "partial", "off"].contains(&landlock.as_str()));
    assert!(d.log().contains(&format!("Landlock: {landlock}")));
}

#[test]
fn the_extension_refuses_a_daemon_that_is_not_the_installed_binary() {
    let dir = tempdir("l0-peer2");
    let d = Daemon::start(&dir);
    let stream = connect(&d.socket());
    let refusal = vase_gnome::peer::check(&stream, std::path::Path::new("/usr/local/libexec/vase/vase-gnome"), false).unwrap_err();
    assert!(matches!(refusal, vase_gnome::peer::Refusal::Exe { .. }), "{refusal}");
}

#[test]
fn a_second_extension_is_turned_away_while_one_is_live() {
    let dir = tempdir("l0-busy");
    let d = Daemon::start(&dir);
    let mut first = Fake::connect(&d.socket());
    first.handshake("boot:1:1", two_windows());
    let mut second = Fake::connect(&d.socket());
    let lines = second.until_eof();
    assert_eq!(lines.last().unwrap()["code"], "busy");
    first.send("ping", json!({}));
    assert_eq!(first.until("pong")[0]["t"], "pong");
}

#[test]
fn a_second_daemon_leaves_a_live_ones_socket_alone() {
    let dir = tempdir("l0-lock");
    let d = Daemon::start(&dir);
    let inode = std::fs::metadata(d.socket()).unwrap().ino();
    let mut second = Daemon { child: Daemon::spawn(&dir), dir: dir.clone() };
    assert_eq!(second.wait(Duration::from_secs(10)), Some(0), "{}", second.log());
    assert!(second.log().contains("another vase-gnome holds"));
    assert_eq!(std::fs::metadata(d.socket()).unwrap().ino(), inode, "the socket file is the first daemon's still");
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
}

#[test]
fn a_restarted_daemon_takes_over_the_stale_socket_and_restores_the_layout() {
    let dir = tempdir("l0-restart");
    let mut d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
    ext.key(ALT_A, &["alt"]);
    ext.key(BACKSLASH, &[]);
    ext.key(108, &[]); // Down: the picker's second row, Files
    ext.key(28, &[]);
    let state = d.state();
    // kill -9 leaves the socket file behind.
    d.child.kill().unwrap();
    d.child.wait().unwrap();
    assert!(d.socket().exists());
    let d = Daemon::start(&dir);
    let c = content();
    let half = c[2] / 2.0;
    let left = [c[0], c[1], half, c[3]];
    let right = [c[0] + half, c[1], half, c[3]];
    let mut ext = Fake::connect(&d.socket());
    let lines = ext.handshake("boot:1:1", json!([window(10, "Ghostty", left), window(11, "Files", right)]));
    assert!(state.exists());
    assert!(places(&lines).is_empty(), "the windows already sit in the saved layout: {:?}", places(&lines));
}

#[test]
fn disconnected_the_daemon_is_frozen_and_the_same_shell_resumes_without_a_move() {
    let dir = tempdir("l0-freeze");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
    drop(ext);
    // The extension is gone (a lock): the daemon saves once and runs nothing.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !d.state().exists() {
        assert!(std::time::Instant::now() < deadline, "no state.json after the extension left:\n{}", d.log());
        std::thread::sleep(Duration::from_millis(20));
    }
    let saved = std::fs::read_to_string(d.state()).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(std::fs::read_to_string(d.state()).unwrap(), saved, "a frozen daemon writes nothing more");

    // The unlock: the same instance; Files closed meanwhile, Ghostty is where vase put it.
    let mut ext = Fake::connect(&d.socket());
    let lines = ext.handshake("boot:1:1", json!([window(10, "Ghostty", content())]));
    assert!(places(&lines).is_empty(), "{:?}", places(&lines));
    let quiet = ext.drain(Duration::from_millis(400));
    assert!(places(&quiet).is_empty(), "nothing moves after a resume: {quiet:?}");
    let state: Value = serde_json::from_str(&std::fs::read_to_string(d.state()).unwrap()).unwrap();
    assert_eq!(state["windows"].as_array().unwrap().len(), 1, "only the window that closed is forgotten: {state}");
}

#[test]
fn a_placement_cut_short_by_a_lock_is_finished_on_the_resume() {
    let dir = tempdir("l0-unfinished");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
    ext.key(ALT_A, &["alt"]);
    let c = content();
    let left = [c[0], c[1], c[2] / 2.0, c[3]];
    assert_eq!(places(&ext.key(BACKSLASH, &[])), vec![(10, left)]);
    // The lock tears the extension down before it applied that place.
    drop(ext);
    std::thread::sleep(Duration::from_millis(200));
    let mut ext = Fake::connect(&d.socket());
    let lines = ext.handshake("boot:1:1", json!([window(10, "Ghostty", c), window(11, "Files", [900.0, 100.0, 800.0, 600.0])]));
    assert_eq!(places(&lines), vec![(10, left)], "the cut-short placement is finished, and nothing else moves");
}

#[test]
fn a_new_shell_gets_the_layout_back_by_app_and_title() {
    let dir = tempdir("l0-relogin");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
    ext.key(ALT_A, &["alt"]);
    ext.key(BACKSLASH, &[]);
    ext.key(108, &[]);
    ext.key(28, &[]);
    drop(ext);
    std::thread::sleep(Duration::from_millis(200));
    // A re-login: a new instance, new window ids, the same apps and titles.
    let mut ext = Fake::connect(&d.socket());
    let mut a = window(10, "Ghostty", [0.0, 32.0, 500.0, 500.0]);
    let mut b = window(11, "Files", [0.0, 32.0, 500.0, 500.0]);
    a["id"] = json!(5010);
    b["id"] = json!(5011);
    let lines = ext.handshake("boot:1:2", json!([a, b]));
    let c = content();
    let half = c[2] / 2.0;
    let mut got = places(&lines);
    got.sort_by_key(|(id, _)| *id);
    assert_eq!(got, vec![(5010, [c[0], c[1], half, c[3]]), (5011, [c[0] + half, c[1], half, c[3]])]);
}

#[test]
fn sigterm_says_bye_saves_and_moves_nothing() {
    let dir = tempdir("l0-term");
    let mut d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
    let _ = std::fs::remove_file(d.state());
    d.signal(libc::SIGTERM);
    let lines = ext.until_eof();
    assert!(places(&lines).is_empty(), "{lines:?}");
    assert_eq!(lines.last().unwrap()["t"], "bye");
    assert_eq!(lines.last().unwrap()["reason"], "stop");
    assert_eq!(d.wait(Duration::from_secs(5)), Some(0));
    assert!(d.state().exists(), "the layout is saved on the way out");
    assert!(!d.socket().exists(), "the socket file goes with the daemon");
}

#[test]
fn the_kill_chord_restores_and_the_daemon_exits() {
    let dir = tempdir("l0-kill");
    let mut d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.handshake("boot:1:1", two_windows());
    ext.send("kill", json!({}));
    let lines = ext.until_eof();
    assert!(places(&lines).contains(&(10, [100.0, 100.0, 800.0, 600.0])), "{lines:?}");
    assert_eq!(lines.last().unwrap()["reason"], "quit");
    assert_eq!(d.wait(Duration::from_secs(5)), Some(0));
}

#[test]
fn the_snapshot_needs_a_display() {
    let dir = tempdir("l0-nodisp");
    let d = Daemon::start(&dir);
    let mut ext = Fake::connect(&d.socket());
    ext.until("hello");
    ext.send("hello", json!({"role": "extension", "instance": "i"}));
    ext.send("snapshot", json!({"displays": [], "windows": []}));
    let lines = ext.until_eof();
    assert_eq!(lines.last().unwrap()["code"], "bad_json");
}
