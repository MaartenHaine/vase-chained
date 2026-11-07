//! Keys on the wire are evdev codes (Clutter's hardware keycode minus 8) at US positions, plus a set
//! of modifiers (protocol §3). vase's `KeyCode` names a key by the character it types unshifted on a
//! US layout, which is the same thing (F22: the only input source is `us`).

use vase_core::input::{Key, KeyCode, Mods};

use crate::protocol::WireKey;

/// Every key vase has a name for, by evdev code.
const KEYS: &[(u32, KeyCode)] = &[
    (1, KeyCode::Escape),
    (2, KeyCode::Char('1')),
    (3, KeyCode::Char('2')),
    (4, KeyCode::Char('3')),
    (5, KeyCode::Char('4')),
    (6, KeyCode::Char('5')),
    (7, KeyCode::Char('6')),
    (8, KeyCode::Char('7')),
    (9, KeyCode::Char('8')),
    (10, KeyCode::Char('9')),
    (11, KeyCode::Char('0')),
    (12, KeyCode::Char('-')),
    (13, KeyCode::Char('=')),
    (14, KeyCode::Backspace),
    (15, KeyCode::Tab),
    (16, KeyCode::Char('q')),
    (17, KeyCode::Char('w')),
    (18, KeyCode::Char('e')),
    (19, KeyCode::Char('r')),
    (20, KeyCode::Char('t')),
    (21, KeyCode::Char('y')),
    (22, KeyCode::Char('u')),
    (23, KeyCode::Char('i')),
    (24, KeyCode::Char('o')),
    (25, KeyCode::Char('p')),
    (26, KeyCode::Char('[')),
    (27, KeyCode::Char(']')),
    (28, KeyCode::Return),
    (30, KeyCode::Char('a')),
    (31, KeyCode::Char('s')),
    (32, KeyCode::Char('d')),
    (33, KeyCode::Char('f')),
    (34, KeyCode::Char('g')),
    (35, KeyCode::Char('h')),
    (36, KeyCode::Char('j')),
    (37, KeyCode::Char('k')),
    (38, KeyCode::Char('l')),
    (39, KeyCode::Char(';')),
    (40, KeyCode::Char('\'')),
    (41, KeyCode::Char('`')),
    (43, KeyCode::Char('\\')),
    (44, KeyCode::Char('z')),
    (45, KeyCode::Char('x')),
    (46, KeyCode::Char('c')),
    (47, KeyCode::Char('v')),
    (48, KeyCode::Char('b')),
    (49, KeyCode::Char('n')),
    (50, KeyCode::Char('m')),
    (51, KeyCode::Char(',')),
    (52, KeyCode::Char('.')),
    (53, KeyCode::Char('/')),
    (57, KeyCode::Char(' ')),
    (96, KeyCode::Return), // keypad Enter
    (103, KeyCode::Up),
    (105, KeyCode::Left),
    (106, KeyCode::Right),
    (108, KeyCode::Down),
];

/// The modifier keys themselves, which the extension must not send (S9) and the daemon ignores if it
/// does: left and right Ctrl, Shift, Alt and Super, and Caps Lock.
const MODIFIERS: &[u32] = &[29, 42, 54, 56, 58, 97, 100, 125, 126];

pub fn is_modifier(code: u32) -> bool {
    MODIFIERS.contains(&code)
}

pub fn key_code(code: u32) -> Option<KeyCode> {
    KEYS.iter().find(|(c, _)| *c == code).map(|(_, k)| *k)
}

pub fn evdev(code: KeyCode) -> Option<u32> {
    KEYS.iter().find(|(_, k)| *k == code).map(|(c, _)| *c)
}

/// The modifier set as vase holds it. `super` is vase's Meta (the Windows key, Command on a Mac).
/// A modifier this build has no name for is ignored.
pub fn mods(names: &[String]) -> Mods {
    let mut m = Mods::default();
    for name in names {
        match name.as_str() {
            "shift" => m.shift = true,
            "ctrl" => m.ctrl = true,
            "alt" => m.alt = true,
            "super" => m.meta = true,
            _ => {}
        }
    }
    m
}

pub fn mod_names(m: Mods) -> Vec<String> {
    [(m.shift, "shift"), (m.ctrl, "ctrl"), (m.alt, "alt"), (m.meta, "super")].iter().filter(|(on, _)| *on).map(|(_, n)| n.to_string()).collect()
}

/// A wire key as vase's `Key`, or `None` for a key vase has no name for.
pub fn key(wire: &WireKey) -> Option<Key> {
    Some(Key { code: key_code(wire.code)?, mods: mods(&wire.mods) })
}

/// vase's `Key` on the wire, or `None` for one with no evdev code.
pub fn wire(key: Key) -> Option<WireKey> {
    Some(WireKey { code: evdev(key.code)?, mods: mod_names(key.mods) })
}

#[cfg(test)]
#[path = "../tests/unit/keys.rs"]
mod tests;
