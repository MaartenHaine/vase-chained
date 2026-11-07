//! Spike S9's stub daemon: the daemon half of the proposed transport (wayfinder §3.5), with no vase-core behind it.
//!
//! It binds its own unix socket (or takes fd 3 from a socket-activation emulator), sends a hello, and answers
//! versioned JSON lines, one connection at a time; on EOF it goes back to accept, like the Disconnected state of §3.1.
//! Throwaway spike code: std only, a minimal JSON reader, no error recovery beyond closing the connection.
//!
//!   s9-stub [--socket PATH | --listen-fd] [--proto N]
//!
//! Messages (one JSON object per line, every one with "v" and "t"):
//!   extension -> daemon: hello {instance, work_area:[x,y,w,h]}, ping {seq}, key {seq, keycode, focus:{id, rect}},
//!                        delta {..} (ignored, counted), ctl {op, n}
//!   daemon -> extension: hello {role, build, pid}, pong {seq, dt_us}, effects {seq, key_mode, effects, draw, dt_us},
//!                        blob {pad}, stats {..}

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

/// Lines longer than this close the connection: the daemon caps what it reads, as the extension must.
const MAX_LINE: u64 = 1 << 20;
/// evdev KEY_BACKSLASH: the prefix command the spike times ("split right", `\`).
const KEY_BACKSLASH: i64 = 43;

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

impl J {
    fn get(&self, k: &str) -> Option<&J> {
        match self {
            J::Obj(m) => m.get(k),
            _ => None,
        }
    }
    fn num(&self, k: &str) -> Option<f64> {
        match self.get(k) {
            Some(J::Num(n)) => Some(*n),
            _ => None,
        }
    }
    fn str(&self, k: &str) -> Option<&str> {
        match self.get(k) {
            Some(J::Str(s)) => Some(s),
            _ => None,
        }
    }
    fn nums(&self, k: &str) -> Option<Vec<f64>> {
        match self.get(k) {
            Some(J::Arr(a)) => a
                .iter()
                .map(|x| match x {
                    J::Num(n) => Some(*n),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }
    fn lit(&mut self, w: &str, v: J) -> Result<J, String> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(format!("bad literal at {}", self.i))
        }
    }
    fn value(&mut self, depth: u32) -> Result<J, String> {
        if depth > 32 {
            return Err("nested too deep".into());
        }
        self.ws();
        match self.s.get(self.i) {
            None => Err("unexpected end".into()),
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(J::Obj(m));
                }
                loop {
                    self.ws();
                    let k = match self.value(depth + 1)? {
                        J::Str(k) => k,
                        _ => return Err("object key is not a string".into()),
                    };
                    self.eat(b':')?;
                    let v = self.value(depth + 1)?;
                    m.insert(k, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(J::Obj(m));
                        }
                        _ => return Err(format!("expected ',' or '}}' at {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(J::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(J::Arr(a));
                        }
                        _ => return Err(format!("expected ',' or ']' at {}", self.i)),
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let mut out = String::new();
                loop {
                    match self.s.get(self.i) {
                        None => return Err("unterminated string".into()),
                        Some(b'"') => {
                            self.i += 1;
                            return Ok(J::Str(out));
                        }
                        Some(b'\\') => {
                            let c = *self.s.get(self.i + 1).ok_or("bad escape")?;
                            self.i += 2;
                            match c {
                                b'n' => out.push('\n'),
                                b't' => out.push('\t'),
                                b'u' => {
                                    let h = std::str::from_utf8(self.s.get(self.i..self.i + 4).ok_or("bad \\u")?)
                                        .map_err(|e| e.to_string())?;
                                    let cp = u32::from_str_radix(h, 16).map_err(|e| e.to_string())?;
                                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                                    self.i += 4;
                                }
                                c => out.push(c as char),
                            }
                        }
                        Some(_) => {
                            let start = self.i;
                            while self.i < self.s.len() && !matches!(self.s[self.i], b'"' | b'\\') {
                                self.i += 1;
                            }
                            out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?);
                        }
                    }
                }
            }
            Some(b't') => self.lit("true", J::Bool(true)),
            Some(b'f') => self.lit("false", J::Bool(false)),
            Some(b'n') => self.lit("null", J::Null),
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
                t.parse::<f64>().map(J::Num).map_err(|_| format!("bad number at {start}"))
            }
        }
    }
}

fn parse(line: &[u8]) -> Result<J, String> {
    let mut p = P { s: line, i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != line.len() {
        return Err(format!("trailing bytes at {}", p.i));
    }
    Ok(v)
}

struct Daemon {
    proto: i64,
    start: Instant,
    work_area: [f64; 4],
    /// Which half the next split puts the focused window in, so every press moves it.
    right: bool,
    draw_bytes: usize,
    split_next: bool,
    delta_bytes: u64,
    lines: u64,
}

impl Daemon {
    fn log(&self, msg: &str) {
        eprintln!("[s9-stub {:>9.3}] {msg}", self.start.elapsed().as_secs_f64());
    }

    fn send(&mut self, out: &mut UnixStream, line: String) -> std::io::Result<()> {
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        if self.split_next && bytes.len() > 2 {
            // A line in two writes 50 ms apart: the reader must join them, not parse half a message.
            self.split_next = false;
            let mid = bytes.len() / 2;
            out.write_all(&bytes[..mid])?;
            out.flush()?;
            std::thread::sleep(Duration::from_millis(50));
            out.write_all(&bytes[mid..])?;
        } else {
            out.write_all(&bytes)?;
        }
        out.flush()
    }

    fn serve(&mut self, stream: UnixStream) {
        self.log("accepted a connection");
        let mut out = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => return self.log(&format!("clone: {e}")),
        };
        let hello = format!(
            r#"{{"v":{},"t":"hello","role":"daemon","build":"s9-stub-{}","pid":{}}}"#,
            self.proto,
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        );
        if let Err(e) = self.send(&mut out, hello) {
            return self.log(&format!("hello: {e}"));
        }
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = match (&mut reader).take(MAX_LINE + 1).read_until(b'\n', &mut buf) {
                Ok(n) => n,
                Err(e) => {
                    self.log(&format!("read: {e}"));
                    break;
                }
            };
            if n == 0 {
                self.log("EOF: the extension went away; back to accept (frozen)");
                break;
            }
            if buf.last() != Some(&b'\n') {
                self.log(&format!("line over {MAX_LINE} bytes or cut off; closing"));
                break;
            }
            let t0 = Instant::now();
            self.lines += 1;
            let msg = match parse(&buf[..buf.len() - 1]) {
                Ok(m) => m,
                Err(e) => {
                    self.log(&format!("protocol error ({e}); closing"));
                    break;
                }
            };
            if msg.num("v") != Some(self.proto as f64) {
                self.log(&format!("version {:?} != {}; refusing", msg.num("v"), self.proto));
                break;
            }
            let reply = match msg.str("t").unwrap_or("") {
                "hello" => {
                    if let Some(w) = msg.nums("work_area").filter(|w| w.len() == 4) {
                        self.work_area = [w[0], w[1], w[2], w[3]];
                    }
                    self.log(&format!(
                        "extension hello: instance {:?}, work area {:?}",
                        msg.str("instance"),
                        self.work_area
                    ));
                    None
                }
                "ping" => Some(format!(
                    r#"{{"v":{},"t":"pong","seq":{},"dt_us":{}}}"#,
                    self.proto,
                    msg.num("seq").unwrap_or(-1.0),
                    t0.elapsed().as_micros()
                )),
                "key" => Some(self.key(&msg, t0)),
                "delta" => {
                    self.delta_bytes += buf.len() as u64;
                    None
                }
                "ctl" => self.ctl(&msg),
                other => {
                    self.log(&format!("unknown message type {other:?}; ignored"));
                    None
                }
            };
            if let Some(line) = reply {
                if let Err(e) = self.send(&mut out, line) {
                    self.log(&format!("write: {e}"));
                    break;
                }
            }
        }
    }

    fn key(&mut self, msg: &J, t0: Instant) -> String {
        let seq = msg.num("seq").unwrap_or(-1.0);
        let focus = msg.get("focus");
        let id = focus.and_then(|f| f.num("id"));
        let mut effects = String::new();
        if msg.num("keycode") == Some(KEY_BACKSLASH as f64) {
            if let Some(id) = id {
                let [x, y, w, h] = self.work_area;
                // Right half, then left 60%: the size changes on every press, so every effect is a real resize
                // (a Wayland configure and ack), not just a move.
                let half = (w / 2.0).floor();
                let (x, w) = if self.right { (x + half, half) } else { (x, (w * 0.6).floor()) };
                self.right = !self.right;
                effects = format!(r#"{{"op":"place","id":{id},"rect":[{x},{y},{w},{h}]}},{{"op":"raise","id":{id}}}"#);
            }
        }
        // A draw list of about draw_bytes bytes stands in for the tab bar and the focus border.
        let draw = "x".repeat(self.draw_bytes);
        format!(
            r#"{{"v":{},"t":"effects","seq":{seq},"key_mode":"idle","effects":[{effects}],"draw":"{draw}","dt_us":{}}}"#,
            self.proto,
            t0.elapsed().as_micros()
        )
    }

    fn ctl(&mut self, msg: &J) -> Option<String> {
        let n = msg.num("n").unwrap_or(0.0) as usize;
        match msg.str("op").unwrap_or("") {
            "split_next" => {
                self.split_next = true;
                None
            }
            "draw_bytes" => {
                self.draw_bytes = n;
                None
            }
            "big_line" => Some(format!(r#"{{"v":{},"t":"blob","pad":"{}"}}"#, self.proto, "y".repeat(n))),
            "garbage" => Some("this is not json".into()),
            "stats" => Some(format!(
                r#"{{"v":{},"t":"stats","lines":{},"delta_bytes":{}}}"#,
                self.proto, self.lines, self.delta_bytes
            )),
            "exit" => {
                self.log("exit requested");
                std::process::exit(3);
            }
            op => {
                self.log(&format!("unknown ctl {op:?}"));
                None
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let proto: i64 = arg("--proto").and_then(|p| p.parse().ok()).unwrap_or(1);
    let mut d = Daemon {
        proto,
        start: Instant::now(),
        work_area: [0.0, 0.0, 1280.0, 720.0],
        right: true,
        draw_bytes: 0,
        split_next: false,
        delta_bytes: 0,
        lines: 0,
    };

    let listener = if args.iter().any(|a| a == "--listen-fd") {
        // Socket activation, as systemd does it: someone else bound and listened, and passed the socket as fd 3.
        if std::env::var("LISTEN_FDS").as_deref() != Ok("1") {
            d.log("--listen-fd without LISTEN_FDS=1");
            std::process::exit(2);
        }
        d.log("using the inherited listening socket on fd 3");
        // SAFETY: fd 3 is the listening socket the emulator passed, and nothing else in this process owns it.
        unsafe { UnixListener::from_raw_fd(3) }
    } else {
        let path = arg("--socket").unwrap_or_else(|| {
            format!("{}/vase/gnome.sock", std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR is unset"))
        });
        let dir = std::path::Path::new(&path).parent().expect("socket path has no parent");
        std::fs::create_dir_all(dir).expect("create the socket dir");
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).expect("chmod 0700 the socket dir");
        // A socket file left by a killed daemon makes bind fail with EADDRINUSE; the daemon owns the path, so it
        // removes it. (The real daemon should hold a lock file first, so two daemons can't steal each other's path.)
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).expect("bind");
        d.log(&format!("bound and listening on {path} (proto v{proto}, pid {})", std::process::id()));
        l
    };

    for conn in listener.incoming() {
        match conn {
            Ok(s) => d.serve(s),
            Err(e) => d.log(&format!("accept: {e}")),
        }
    }
}
