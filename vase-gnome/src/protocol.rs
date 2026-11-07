//! The wire, protocol v1 (`docs/linux-tiler/protocol-v1.md`): framing, the version and the line cap,
//! and the messages the extension sends, parsed into types. What the daemon sends is built with
//! `out`, which stamps `v`, `t` and `seq` on every line.

use serde::Deserialize;
use serde_json::{json, Map, Value};
use vase_core::geometry::Rect;

/// The only version this build speaks. Every line carries it (§2).
pub const VERSION: u64 = 1;

/// The most bytes a line may hold before its `\n`, in both directions (§2).
pub const MAX_LINE: usize = 1 << 20;

/// A `draw` line should stay under this (S9: 16 KB per reply cost nothing measurable) ...
pub const DRAW_BUDGET: usize = 16 * 1024;

/// ... and must stay under this: past it the daemon drops list rows until it fits (§2).
pub const DRAW_LIMIT: usize = 64 * 1024;

/// How many bytes the daemon reads from the socket at a time, as the extension's chunk reader does.
pub const CHUNK: usize = 64 * 1024;

/// Splits a byte stream into lines, refusing a line as soon as more than `MAX_LINE` bytes of it have
/// arrived, so an oversized line is never held whole (§2, S9).
#[derive(Default)]
pub struct LineReader {
    pending: Vec<u8>,
}

/// A line passed the cap.
#[derive(Debug, PartialEq, Eq)]
pub struct TooLong;

impl LineReader {
    /// Feed one chunk; returns the lines it completed, without their `\n`.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, TooLong> {
        let mut lines = Vec::new();
        let mut rest = chunk;
        while let Some(i) = rest.iter().position(|b| *b == b'\n') {
            if self.pending.len() + i > MAX_LINE {
                return Err(TooLong);
            }
            let mut line = std::mem::take(&mut self.pending);
            line.extend_from_slice(&rest[..i]);
            lines.push(line);
            rest = &rest[i + 1..];
        }
        if self.pending.len() + rest.len() > MAX_LINE {
            return Err(TooLong);
        }
        self.pending.extend_from_slice(rest);
        Ok(lines)
    }
}

/// A rect on the wire: `[x, y, w, h]` in global stage pixels.
pub type WireRect = [f64; 4];

pub fn rect(r: WireRect) -> Rect {
    Rect::new(r[0], r[1], r[2], r[3])
}

pub fn wire_rect(r: Rect) -> WireRect {
    [r.x, r.y, r.w, r.h]
}

/// A key: the evdev code and the modifiers held (§3).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WireKey {
    pub code: u32,
    #[serde(default)]
    pub mods: Vec<String>,
}

impl WireKey {
    pub fn to_json(&self) -> Value {
        json!({"code": self.code, "mods": self.mods})
    }
}

/// One monitor (§3).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WireDisplay {
    pub id: u32,
    #[serde(default)]
    pub connector: String,
    pub bounds: WireRect,
    pub work_area: WireRect,
    #[serde(default = "one")]
    pub scale: f64,
}

fn one() -> f64 {
    1.0
}

fn yes() -> bool {
    true
}

/// A window object, as the snapshot lists it and a `window` delta carries it at the top level of its
/// line (§3). `frame` is left out of a delta while a placement of the window is in flight (§5.2).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WireWindow {
    pub id: u64,
    #[serde(default)]
    pub pid: i32,
    pub app: String,
    #[serde(default)]
    pub app_id: String,
    pub title: String,
    #[serde(default)]
    pub frame: Option<WireRect>,
    #[serde(default)]
    pub layer: i64,
    #[serde(default = "yes")]
    pub tileable: bool,
    #[serde(default)]
    pub minimized: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default = "yes")]
    pub on_workspace: bool,
    #[serde(default)]
    pub x11: bool,
}

/// An installed app the picker can launch: its display name and its desktop-file id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WireApp {
    pub app: String,
    pub app_id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hello {
    pub role: String,
    pub instance: String,
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub shell: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Snapshot {
    pub displays: Vec<WireDisplay>,
    #[serde(default)]
    pub primary: usize,
    pub windows: Vec<WireWindow>,
    #[serde(default)]
    pub focus: Option<u64>,
    /// The installed apps the picker offers (`Shell.AppSystem`), optional.
    #[serde(default)]
    pub apps: Vec<WireApp>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Measured {
    pub text: String,
    pub size: f64,
    pub advances: Vec<f64>,
}

/// What the extension sends (§5), parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Hello(Hello),
    Snapshot(Snapshot),
    Window(WireWindow),
    WindowGone {
        id: u64,
    },
    Stacking {
        ids: Vec<u64>,
    },
    Focus {
        id: Option<u64>,
    },
    Displays {
        displays: Vec<WireDisplay>,
        primary: usize,
    },
    Key {
        key: WireKey,
        via: String,
    },
    Click {
        at: [f64; 2],
        button: u32,
        target: String,
    },
    End {
        what: String,
        reason: String,
    },
    Kill,
    Pause,
    Measure {
        texts: Vec<Measured>,
    },
    Settled {
        id: u64,
        re: u64,
        frame: WireRect,
    },
    Ping,
    /// A `t` this build doesn't know: answered with a non-fatal `unknown_type`.
    Unknown(String),
}

/// One parsed line: the sender's `seq`, its `ack` (the newest effects batch it finished applying)
/// and the message.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub seq: u64,
    pub ack: u64,
    pub msg: Incoming,
}

/// Why a line was refused. Both are fatal (§10).
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// Not a JSON object, or a field missing or of the wrong type.
    BadJson(String),
    /// A `v` other than 1.
    Version(Value),
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::BadJson(_) => "bad_json",
            Refusal::Version(_) => "version",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Refusal::BadJson(why) => why.clone(),
            Refusal::Version(v) => format!("speaks v{v}, this daemon v{VERSION}"),
        }
    }
}

#[derive(Deserialize)]
struct Id {
    id: u64,
}

#[derive(Deserialize)]
struct Ids {
    ids: Vec<u64>,
}

#[derive(Deserialize)]
struct FocusId {
    id: Option<u64>,
}

#[derive(Deserialize)]
struct DisplaysMsg {
    displays: Vec<WireDisplay>,
    #[serde(default)]
    primary: usize,
}

#[derive(Deserialize)]
struct KeyMsg {
    key: WireKey,
    #[serde(default)]
    via: String,
}

#[derive(Deserialize)]
struct ClickMsg {
    at: [f64; 2],
    #[serde(default = "first_button")]
    button: u32,
    target: String,
}

fn first_button() -> u32 {
    1
}

#[derive(Deserialize)]
struct EndMsg {
    what: String,
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct MeasureMsg {
    texts: Vec<Measured>,
}

#[derive(Deserialize)]
struct SettledMsg {
    id: u64,
    re: u64,
    frame: WireRect,
}

fn field<T: serde::de::DeserializeOwned>(t: &str, obj: Value) -> Result<T, Refusal> {
    serde_json::from_value(obj).map_err(|e| Refusal::BadJson(format!("{t}: {e}")))
}

/// Parse one line. The version is checked before anything else, so a v2 peer is told so even if
/// its line would not parse as v1.
pub fn parse(line: &[u8]) -> Result<Line, Refusal> {
    let value: Value = serde_json::from_slice(line).map_err(|e| Refusal::BadJson(format!("not JSON: {e}")))?;
    let Value::Object(map) = &value else { return Err(Refusal::BadJson("not a JSON object".into())) };
    match map.get("v") {
        Some(v) if v.as_u64() == Some(VERSION) => {}
        Some(v) => return Err(Refusal::Version(v.clone())),
        None => return Err(Refusal::BadJson("no v".into())),
    }
    let t = map.get("t").and_then(Value::as_str).ok_or_else(|| Refusal::BadJson("no t".into()))?.to_string();
    let seq = map.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let ack = map.get("ack").and_then(Value::as_u64).unwrap_or(0);
    let msg = match t.as_str() {
        "hello" => Incoming::Hello(field(&t, value)?),
        "snapshot" => Incoming::Snapshot(field(&t, value)?),
        "window" => Incoming::Window(field(&t, value)?),
        "window_gone" => Incoming::WindowGone { id: field::<Id>(&t, value)?.id },
        "stacking" => Incoming::Stacking { ids: field::<Ids>(&t, value)?.ids },
        "focus" => Incoming::Focus { id: field::<FocusId>(&t, value)?.id },
        "displays" => {
            let m: DisplaysMsg = field(&t, value)?;
            Incoming::Displays { displays: m.displays, primary: m.primary }
        }
        "key" => {
            let m: KeyMsg = field(&t, value)?;
            Incoming::Key { key: m.key, via: m.via }
        }
        "click" => {
            let m: ClickMsg = field(&t, value)?;
            Incoming::Click { at: m.at, button: m.button, target: m.target }
        }
        "end" => {
            let m: EndMsg = field(&t, value)?;
            Incoming::End { what: m.what, reason: m.reason }
        }
        "kill" => Incoming::Kill,
        "pause" => Incoming::Pause,
        "measure" => Incoming::Measure { texts: field::<MeasureMsg>(&t, value)?.texts },
        "settled" => {
            let m: SettledMsg = field(&t, value)?;
            Incoming::Settled { id: m.id, re: m.re, frame: m.frame }
        }
        "ping" => Incoming::Ping,
        _ => Incoming::Unknown(t),
    };
    Ok(Line { seq, ack, msg })
}

/// One line the daemon sends: `v`, `t` and `seq` stamped on `fields`, then the `\n`.
pub fn out(t: &str, seq: u64, fields: Value) -> String {
    let mut map = match fields {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("v".into(), json!(VERSION));
    map.insert("t".into(), json!(t));
    map.insert("seq".into(), json!(seq));
    let mut line = Value::Object(map).to_string();
    line.push('\n');
    line
}

/// The `error` line (§10).
pub fn error(seq: u64, re: Option<u64>, code: &str, fatal: bool, message: &str) -> String {
    let mut fields = json!({"code": code, "fatal": fatal, "message": message});
    if let Some(re) = re {
        fields["re"] = json!(re);
    }
    out("error", seq, fields)
}

#[cfg(test)]
#[path = "../tests/unit/protocol.rs"]
mod tests;
