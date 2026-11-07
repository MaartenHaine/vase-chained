//! Measuring from Pango's arrays, and draw lines that carry only what changed.

use super::*;

fn measured(text: &str, size: f64, step: f64) -> Measured {
    Measured { text: text.into(), size, advances: (1..=text.chars().count()).map(|i| i as f64 * step).collect() }
}

#[test]
fn a_title_is_as_wide_as_its_last_advance() {
    let mut m = Measurer::default();
    m.add(measured("Files", 12.0, 7.0));
    assert_eq!(m.measure("Files", 12.0), (35.0, true));
    // Another size is another measurement.
    assert!(!m.measure("Files", 13.0).1);
}

#[test]
fn an_ellipsized_prefix_is_the_prefix_plus_the_ellipsis() {
    let mut m = Measurer::default();
    m.add(measured("~/src/vase-chained", 12.0, 7.0));
    m.add(measured("…", 12.0, 9.0));
    assert_eq!(m.measure("~/src…", 12.0), (5.0 * 7.0 + 9.0, true));
    assert_eq!(m.measure("…", 12.0), (9.0, true));
}

#[test]
fn other_text_sums_the_glyph_table_and_guesses_what_it_lacks() {
    let mut m = Measurer::default();
    for c in "12 ".chars() {
        m.add(measured(&c.to_string(), 12.0, 6.5));
    }
    assert_eq!(m.measure("12 ", 12.0), (19.5, true));
    let (w, exact) = m.measure("1x", 12.0);
    assert!(!exact);
    assert_eq!(w, 6.5 + 12.0 * GUESS);
}

#[test]
fn a_measurement_of_the_wrong_length_is_ignored() {
    let mut m = Measurer::default();
    m.add(Measured { text: "abc".into(), size: 12.0, advances: vec![1.0] });
    assert!(!m.measure("abc", 12.0).1);
}

#[test]
fn a_draw_carries_only_what_changed_and_null_for_what_went() {
    let mut s = Surfaces::default();
    s.set("border", json!([0, 0, 10, 10]));
    let first = s.take().unwrap();
    assert_eq!(first["border"], json!([0, 0, 10, 10]));
    assert_eq!(first["bar"], Value::Null, "the first draw names every surface");
    s.set("border", json!([0, 0, 10, 10]));
    assert!(s.take().is_none(), "nothing changed, nothing sent");
    s.set("border", Value::Null);
    assert_eq!(s.take().unwrap().len(), 1);
    // A new connection gets everything again.
    s.forget_sent();
    assert_eq!(s.take().unwrap().len(), SURFACES.len());
}

#[test]
fn an_oversized_draw_loses_list_rows_from_the_bottom() {
    let rows: Vec<Value> = (0..2000).map(|i| json!({"number": i, "label": "x".repeat(60)})).collect();
    let mut fields = Map::new();
    fields.insert("list".into(), json!({"rows": rows}));
    fit(&mut fields);
    let len = Value::Object(fields.clone()).to_string().len();
    assert!(len <= DRAW_LIMIT, "{len}");
    let kept = fields["list"]["rows"].as_array().unwrap();
    assert!(kept.len() > 100);
    assert_eq!(kept[0]["number"], 0, "the top rows stay");
}
