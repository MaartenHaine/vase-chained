//! A fake extension: the GNOME Shell extension's half of protocol v1, played by a test over the real
//! socket against the real `vase-gnome` binary (wayfinder §5 M1a, protocol §11). It does what the
//! extension will: the peer check on the daemon, hello, the measurements, the snapshot, then keys and
//! deltas, acknowledging each effects batch as it arrives.

#![allow(dead_code)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vase_gnome::peer;
use vase_gnome::protocol::LineReader;

/// The daemon under test: the built binary, with its XDG dirs in a private temp dir.
pub struct Daemon {
    pub child: Child,
    pub dir: PathBuf,
}

pub fn daemon_exe() -> PathBuf {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_vase-gnome")).unwrap()
}

/// A fresh, empty directory for one test.
pub fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Daemon {
    pub fn start(dir: &Path) -> Daemon {
        let child = Daemon::spawn(dir);
        let mut d = Daemon { child, dir: dir.to_path_buf() };
        d.wait_for_socket();
        d
    }

    pub fn spawn(dir: &Path) -> Child {
        // A session always has its runtime dir; the daemon makes only its own `vase/` in it.
        std::fs::create_dir_all(dir.join("run")).unwrap();
        let log = File::options().create(true).append(true).open(dir.join("daemon.log")).unwrap();
        Command::new(daemon_exe())
            .env_clear()
            .env("HOME", dir)
            .env("XDG_RUNTIME_DIR", dir.join("run"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_STATE_HOME", dir.join("state"))
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap()
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.join("run/vase/gnome.sock")
    }

    pub fn state(&self) -> PathBuf {
        self.dir.join("state/vase/state.json")
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
    }

    /// Wait for the socket file (no probe connection: it would be the daemon's one live connection).
    fn wait_for_socket(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.socket().exists() {
            let status = self.child.try_wait().unwrap();
            assert!(Instant::now() < deadline && status.is_none(), "the daemon never bound ({status:?}):\n{}", self.log());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn signal(&self, signal: i32) {
        // SAFETY: a signal to our own child.
        assert_eq!(unsafe { libc::kill(self.child.id() as i32, signal) }, 0);
    }

    /// Wait for the daemon to exit; its exit code.
    pub fn wait(&mut self, within: Duration) -> Option<i32> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            if Instant::now() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The extension's side of one connection.
pub struct Fake {
    pub stream: UnixStream,
    reader: LineReader,
    pending: Vec<Value>,
    pub seq: u64,
    pub ack: u64,
    pub eof: bool,
}

/// Pango, as the fake fakes it: 7 px per character at every size.
pub fn advances(text: &str) -> Vec<f64> {
    (1..=text.chars().count()).map(|i| i as f64 * 7.0).collect()
}

pub fn window(id: u64, app: &str, frame: [f64; 4]) -> Value {
    json!({"id": id, "pid": id, "app": app, "app_id": format!("org.example.{app}.desktop"), "title": format!("{app} {id}"), "frame": frame})
}

/// One 1920x1080 monitor at the origin with GNOME's 32 px top panel.
pub fn displays() -> Value {
    json!([{"id": 7, "connector": "HDMI-1", "bounds": [0, 0, 1920, 1080], "work_area": [0, 32, 1920, 1048], "scale": 1.0}])
}

/// Connect, retrying the moment between the socket file appearing and the daemon listening.
pub fn connect(socket: &Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(socket) {
            Ok(s) => return s,
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("connect {}: {e}", socket.display()),
        }
    }
}

impl Fake {
    /// Connect, after the peer check the extension runs on the daemon (S9).
    pub fn connect(socket: &Path) -> Fake {
        let stream = connect(socket);
        peer::check(&stream, &daemon_exe(), false).expect("the daemon passes the extension's peer check");
        Fake::over(stream)
    }

    pub fn over(stream: UnixStream) -> Fake {
        stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        Fake { stream, reader: LineReader::default(), pending: Vec::new(), seq: 0, ack: 0, eof: false }
    }

    pub fn send(&mut self, t: &str, mut fields: Value) {
        self.seq += 1;
        fields["v"] = json!(1);
        fields["t"] = json!(t);
        fields["seq"] = json!(self.seq);
        fields["ack"] = json!(self.ack);
        let mut line = fields.to_string();
        line.push('\n');
        self.stream.write_all(line.as_bytes()).unwrap();
    }

    pub fn send_raw(&mut self, raw: &[u8]) {
        self.stream.write_all(raw).unwrap();
    }

    /// Read what has arrived, for up to `within`; stops early at EOF.
    fn fill(&mut self, within: Duration) {
        let deadline = Instant::now() + within;
        let mut buf = [0u8; 65536];
        while Instant::now() < deadline && !self.eof {
            match self.stream.read(&mut buf) {
                Ok(0) => self.eof = true,
                Ok(n) => {
                    for line in self.reader.push(&buf[..n]).expect("the daemon keeps to the line cap") {
                        let v: Value = serde_json::from_slice(&line).unwrap();
                        assert_eq!(v["v"], 1, "every line carries v: {v}");
                        // Every batch is applied as soon as it arrives.
                        if v["t"] == "effects" {
                            self.ack = v["seq"].as_u64().unwrap();
                        }
                        self.pending.push(v);
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => self.eof = true,
            }
        }
    }

    /// Every line up to and including the first of type `t`.
    pub fn until(&mut self, t: &str) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(i) = self.pending.iter().position(|v| v["t"] == t) {
                return self.pending.drain(..=i).collect();
            }
            assert!(Instant::now() < deadline && !self.eof, "no {t} line; got {:?}", self.pending);
            self.fill(Duration::from_millis(50));
        }
    }

    /// Everything that arrives within `within`.
    pub fn drain(&mut self, within: Duration) -> Vec<Value> {
        self.fill(within);
        std::mem::take(&mut self.pending)
    }

    /// Wait for the daemon to close the connection; the lines it sent first.
    pub fn until_eof(&mut self) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.eof {
            assert!(Instant::now() < deadline, "the daemon kept the connection open; got {:?}", self.pending);
            self.fill(Duration::from_millis(50));
        }
        std::mem::take(&mut self.pending)
    }

    /// hello, the measurements, the snapshot: the daemon's answer, up to its first mode line.
    pub fn handshake(&mut self, instance: &str, windows: Value) -> Vec<Value> {
        let hello = self.until("hello");
        assert_eq!(hello[0]["role"], "daemon");
        self.send("hello", json!({"role": "extension", "instance": instance, "build": "test", "shell": "50.1"}));
        let mut texts: Vec<String> = hello[0]["glyphs"].as_str().unwrap().chars().map(String::from).collect();
        texts.extend(windows.as_array().unwrap().iter().map(|w| w["title"].as_str().unwrap().to_string()));
        let entries: Vec<Value> = hello[0]["sizes"].as_array().unwrap().iter().flat_map(|s| texts.iter().map(move |t| json!({"text": t, "size": s, "advances": advances(t)}))).collect();
        self.send("measure", json!({"texts": entries}));
        self.send("snapshot", json!({"displays": displays(), "primary": 0, "windows": windows, "focus": null}));
        let mut lines = hello;
        lines.extend(self.until("mode"));
        assert!(lines.iter().any(|l| l["t"] == "grabs"), "the answer to the snapshot starts with the grabs: {lines:?}");
        lines
    }

    pub fn key(&mut self, code: u32, mods: &[&str]) -> Vec<Value> {
        self.send("key", json!({"key": {"code": code, "mods": mods}, "time_us": 1, "via": "grab"}));
        self.until("mode")
    }
}

pub fn of<'a>(lines: &'a [Value], t: &str) -> Vec<&'a Value> {
    lines.iter().filter(|l| l["t"] == t).collect()
}

pub fn places(lines: &[Value]) -> Vec<(u64, [f64; 4])> {
    of(lines, "effects")
        .iter()
        .flat_map(|l| l["effects"].as_array().unwrap().iter())
        .filter(|e| e["k"] == "place")
        .map(|e| {
            let r: Vec<f64> = e["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            (e["id"].as_u64().unwrap(), [r[0], r[1], r[2], r[3]])
        })
        .collect()
}
