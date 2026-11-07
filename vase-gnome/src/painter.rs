//! `GnomePainter`: vase-core's `Painter` as data (protocol §6.7). Each call sets one surface of the
//! chrome; the host sends what changed as one `draw` line after each line it answers. Text is
//! measured from the Pango advance arrays the extension sends (§5.6), so the bar ellipsizes titles
//! as the extension will draw them.

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde_json::{json, Map, Value};
use vase_core::chrome::bar::{Bar, Hits};
use vase_core::chrome::help::{CellKind, HelpLayout, TextStyle};
use vase_core::chrome::powerline::{self, LeadGlyph, TAB_ICON};
use vase_core::chrome::theme::{self, Role};
use vase_core::chrome::{BarHits, ListAt, Painter, Position, SwitchRow, FONT_SIZE};
use vase_core::geometry::Rect;

use crate::backend::SharedRef;
use crate::protocol::{wire_rect, Measured, DRAW_LIMIT};

/// The surfaces a `draw` line can carry, in the order the extension stacks them.
pub const SURFACES: [&str; 6] = ["bar", "stack_bars", "panes", "border", "list", "help"];

/// The font sizes the daemon measures at: the chrome's text, and a glyph mark one point larger
/// (`powerline::lead_pill`). Sent in the hello.
pub const SIZES: [f64; 2] = [FONT_SIZE, FONT_SIZE + 1.0];

/// The characters measured once per connection (§4): digits, the space, the ellipsis, the marks.
pub const GLYPHS: &str = "0123456789 …∗◇★";

/// How many measured strings are kept before the oldest go. A title is measured again with its next
/// delta, so an evicted one costs a guess until then.
const KEEP: usize = 4096;

/// A guess for a character nobody has measured: about a sans-serif average.
const GUESS: f64 = 0.6;

fn size_key(size: f64) -> u32 {
    (size * 100.0).round() as u32
}

/// Widths from Pango.
#[derive(Default)]
pub struct Measurer {
    texts: HashMap<(u32, String), Vec<f64>>,
    order: VecDeque<(u32, String)>,
    glyphs: HashMap<(u32, char), f64>,
}

impl Measurer {
    pub fn add(&mut self, m: Measured) {
        let size = size_key(m.size);
        let chars: Vec<char> = m.text.chars().collect();
        if chars.len() != m.advances.len() {
            eprintln!("vase-gnome: measure for {:?} has {} advances for {} chars; ignored", m.text, m.advances.len(), chars.len());
            return;
        }
        if chars.len() == 1 {
            self.glyphs.insert((size, chars[0]), m.advances[0]);
        }
        let key = (size, m.text);
        if self.texts.insert(key.clone(), m.advances).is_none() {
            self.order.push_back(key);
            while self.order.len() > KEEP {
                if let Some(old) = self.order.pop_front() {
                    self.texts.remove(&old);
                }
            }
        }
    }

    fn glyph(&self, c: char, size: f64) -> Option<f64> {
        self.glyphs.get(&(size_key(size), c)).copied()
    }

    /// The width of `text`, and whether it came from Pango rather than a guess.
    pub fn measure(&self, text: &str, size: f64) -> (f64, bool) {
        if text.is_empty() {
            return (0.0, true);
        }
        let key = size_key(size);
        if let Some(adv) = self.texts.get(&(key, text.to_string())) {
            return (adv.last().copied().unwrap_or(0.0), true);
        }
        // The bar ellipsizes a title by measuring its prefixes with "…" on the end (chrome::bar).
        if let Some(base) = text.strip_suffix('…') {
            let n = base.chars().count();
            let prefix = if n == 0 { Some(0.0) } else { self.texts.iter().find(|((k, t), adv)| *k == key && adv.len() > n && t.starts_with(base)).map(|(_, adv)| adv[n - 1]) };
            if let (Some(w), Some(dots)) = (prefix, self.glyph('…', size)) {
                return (w + dots, true);
            }
        }
        let mut exact = true;
        let w = text
            .chars()
            .map(|c| {
                self.glyph(c, size).unwrap_or_else(|| {
                    exact = false;
                    size * GUESS
                })
            })
            .sum();
        (w, exact)
    }
}

/// The chrome as the daemon wants it drawn, and what was last sent.
#[derive(Default)]
pub struct Surfaces {
    now: BTreeMap<&'static str, Value>,
    sent: BTreeMap<&'static str, Value>,
    pub measurer: Measurer,
    /// Texts the daemon measured by guess, to ask the extension for (`measure_request`).
    pub requests: BTreeMap<(u32, String), f64>,
}

impl Surfaces {
    fn set(&mut self, surface: &'static str, value: Value) {
        self.now.insert(surface, value);
    }

    /// The `draw` fields that changed since the last one sent: an object to draw, `null` to hide.
    pub fn take(&mut self) -> Option<Map<String, Value>> {
        let mut out = Map::new();
        for s in SURFACES {
            let now = self.now.get(s).cloned().unwrap_or(Value::Null);
            if self.sent.get(s) != Some(&now) {
                out.insert(s.to_string(), now.clone());
                self.sent.insert(s, now);
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// A new connection's extension has drawn nothing yet.
    pub fn forget_sent(&mut self) {
        self.sent.clear();
        self.requests.clear();
    }

    pub fn take_requests(&mut self) -> Vec<(String, f64)> {
        std::mem::take(&mut self.requests).into_iter().map(|((_, text), size)| (text, size)).collect()
    }
}

/// Keep a `draw` line under the hard limit by dropping list rows from the bottom (§2).
pub fn fit(fields: &mut Map<String, Value>) {
    let len = |f: &Map<String, Value>| Value::Object(f.clone()).to_string().len();
    let mut warned = false;
    while len(fields) > DRAW_LIMIT {
        let Some(rows) = fields.get_mut("list").and_then(|l| l.get_mut("rows")).and_then(Value::as_array_mut) else { break };
        if rows.pop().is_none() {
            break;
        }
        if !warned {
            eprintln!("vase-gnome: a draw line passed {DRAW_LIMIT} bytes; dropping list rows to fit");
            warned = true;
        }
    }
}

pub struct GnomePainter(pub SharedRef);

fn hex(role: Role) -> String {
    let c = theme::palette().color(role);
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]), b(c[3]))
}

impl GnomePainter {
    /// The desktop-file id of an app, for the extension's icon lookup; the display name if none is known.
    fn icon(&self, app: &str) -> String {
        self.0.borrow().mirror.app_id(app).unwrap_or_else(|| app.to_string())
    }

    /// One bar, laid out in the powerline style, as draw items in global coordinates.
    fn bar_object(&self, bar: &Bar) -> (Value, BarHits) {
        let measure = |t: &str, s: f64| self.measure(t, s);
        let layout = powerline::layout(bar, &theme::mark(), &measure);
        let (ox, oy, h) = (bar.rect.x, bar.rect.y, bar.rect.h);
        let mut items = vec![json!({"k": "rect", "r": wire_rect(bar.rect), "fill": hex(Role::Bg), "radius": 0})];
        if let Some(lead) = &layout.lead {
            items.push(json!({"k": "rect", "r": [ox, oy, lead.width, h], "fill": hex(Role::Accent), "radius": layout.radius}));
            if let LeadGlyph::Glyph { x, text, size } = &lead.glyph {
                items.push(json!({"k": "text", "at": [ox + x, oy], "text": text, "size": size, "color": hex(Role::Bg), "bold": true}));
            }
        }
        for tab in &layout.tabs {
            items.push(json!({"k": "rect", "r": [ox + tab.x0, oy, tab.x1 - tab.x0 + layout.radius, h], "fill": hex(tab.fill), "radius": layout.radius}));
            for run in &tab.content {
                match run {
                    vase_core::chrome::bar::Run::Text { x, text, color } => {
                        items.push(json!({"k": "text", "at": [ox + x, oy], "text": text, "size": FONT_SIZE, "color": hex(*color), "bold": false}));
                    }
                    vase_core::chrome::bar::Run::Icon { x, app, .. } => {
                        items.push(json!({"k": "icon", "r": [ox + x, oy + (h - TAB_ICON) / 2.0, TAB_ICON, TAB_ICON], "app_id": self.icon(app)}));
                    }
                }
            }
        }
        if let Some((x, armed)) = layout.dot {
            let d = powerline::DOT_D;
            items.push(json!({"k": "rect", "r": [ox + x, oy + (h - d) / 2.0, d, d], "fill": hex(if armed { Role::Accent } else { Role::Dim }), "radius": d / 2.0}));
        }
        let hits = BarHits { tabs: layout.hits(), apps: layout.app_hits() };
        let value = json!({"rect": wire_rect(bar.rect), "items": items, "hits": hits.tabs.iter().map(|(a, b)| [*a, *b]).collect::<Vec<_>>()});
        (value, hits)
    }

    fn set(&mut self, surface: &'static str, value: Value) {
        self.0.borrow_mut().surfaces.set(surface, value);
    }
}

impl Painter for GnomePainter {
    fn measure(&self, text: &str, size: f64) -> f64 {
        let (w, exact) = self.0.borrow().surfaces.measurer.measure(text, size);
        // A prefix with "…" is an ellipsizing probe: the whole title gets asked for, not each probe.
        if !exact && !text.ends_with('…') {
            self.0.borrow_mut().surfaces.requests.insert((size_key(size), text.to_string()), size);
        }
        w
    }

    fn bar(&mut self, bar: &Bar) -> BarHits {
        let (value, hits) = self.bar_object(bar);
        self.set("bar", value);
        hits
    }

    fn prompt(&mut self, rect: Rect, position: Position, text: &str) {
        let edge = match position {
            Position::Top => "top",
            Position::Bottom => "bottom",
        };
        self.set("bar", json!({"rect": wire_rect(rect), "prompt": text, "edge": edge, "items": [], "hits": []}));
    }

    fn hide_bar(&mut self) {
        self.set("bar", Value::Null);
    }

    fn stack_bars(&mut self, bars: &[Bar]) -> Vec<Hits> {
        let (values, hits): (Vec<Value>, Vec<Hits>) = bars.iter().map(|b| self.bar_object(b)).map(|(v, h)| (v, h.tabs)).unzip();
        self.set("stack_bars", if values.is_empty() { Value::Null } else { Value::Array(values) });
        hits
    }

    fn panes(&mut self, panes: &[(Rect, bool)]) {
        let value: Vec<Value> = panes.iter().map(|(r, focused)| json!({"rect": wire_rect(*r), "focused": focused})).collect();
        self.set("panes", if value.is_empty() { Value::Null } else { Value::Array(value) });
    }

    fn focus_border(&mut self, rect: Option<Rect>) {
        self.set("border", rect.map_or(Value::Null, |r| json!(wire_rect(r))));
    }

    fn list(&mut self, at: ListAt, header: &str, rows: &[SwitchRow], selected: usize) {
        let (rect, centered) = match at {
            ListAt::Centered(r) => (r, true),
            ListAt::Filling(r) => (r, false),
        };
        let rows: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({"number": r.number, "prefix": r.prefix, "icons": r.icons.iter().map(|a| self.icon(a)).collect::<Vec<_>>(), "label": r.label,
                       "dim": r.dim, "off_workspace": r.off_workspace, "favorite": r.favorite, "current": r.current})
            })
            .collect();
        self.set("list", json!({"rect": wire_rect(rect), "centered": centered, "header": header, "rows": rows, "selected": selected}));
    }

    fn hide_list(&mut self) {
        self.set("list", Value::Null);
    }

    fn help(&mut self, layout: &HelpLayout) {
        let style = |s: TextStyle| match s {
            TextStyle::Title => "title",
            TextStyle::Section => "section",
            TextStyle::Keys => "keys",
            TextStyle::Label => "label",
        };
        let kind = |k: CellKind| match k {
            CellKind::Plain => "plain",
            CellKind::Active => "active",
            CellKind::Ghost => "ghost",
        };
        let texts: Vec<Value> = layout.texts.iter().map(|t| json!({"rect": wire_rect(t.rect), "text": t.text, "style": style(t.style)})).collect();
        let cells: Vec<Value> = layout.cells.iter().map(|c| json!({"rect": wire_rect(c.rect), "kind": kind(c.kind)})).collect();
        self.set("help", json!({"rect": wire_rect(layout.rect), "texts": texts, "cells": cells}));
    }

    fn hide_help(&mut self) {
        self.set("help", Value::Null);
    }

    fn hide_bars(&mut self) {
        self.set("bar", Value::Null);
        self.set("stack_bars", Value::Null);
    }

    fn hide_all(&mut self) {
        for s in SURFACES {
            self.set(s, Value::Null);
        }
    }

    /// The extension resolves icons itself, from the desktop-file id in each draw item.
    fn prewarm_icon(&mut self, _app: &str) {}
}

#[cfg(test)]
#[path = "../tests/unit/painter.rs"]
mod tests;
