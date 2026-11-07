//! evdev codes at US positions, and the modifier names (protocol §3).

use super::*;

#[test]
fn every_named_key_maps_back_to_its_code() {
    for (code, key) in KEYS {
        // Keypad Enter types Return too; Return goes back as the main Enter.
        let back = evdev(*key).unwrap();
        assert!(back == *code || (*key == KeyCode::Return && back == 28), "{code} {key:?}");
        assert!(!is_modifier(*code));
    }
}

#[test]
fn the_prefixes_are_alt_a_and_alt_e() {
    assert_eq!(wire(vase_core::input::prefix()).unwrap(), WireKey { code: 30, mods: vec!["alt".into()] });
    assert_eq!(wire(vase_core::input::stack_prefix()).unwrap(), WireKey { code: 18, mods: vec!["alt".into()] });
}

#[test]
fn super_is_vases_meta_and_unknown_names_are_ignored() {
    let m = mods(&["super".into(), "shift".into(), "hyper".into()]);
    assert!(m.meta && m.shift && !m.alt && !m.ctrl);
    assert_eq!(mod_names(m), vec!["shift", "super"]);
}

#[test]
fn a_key_vase_has_no_name_for_is_none() {
    assert_eq!(key(&WireKey { code: 59, mods: vec![] }), None); // F1
    assert!(is_modifier(56)); // Left Alt
}
