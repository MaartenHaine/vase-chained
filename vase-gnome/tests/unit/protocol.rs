//! Framing: the line cap, the version on every line, and parse errors (protocol §2, §10).

use super::*;

#[test]
fn lines_split_across_chunks_come_out_whole() {
    let mut r = LineReader::default();
    assert_eq!(r.push(b"{\"a\"").unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(r.push(b":1}\n{\"b\":2}\n{").unwrap(), vec![b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec()]);
    assert_eq!(r.push(b"}\n").unwrap(), vec![b"{}".to_vec()]);
}

#[test]
fn a_line_of_exactly_the_cap_passes_and_one_byte_more_is_refused_before_its_end() {
    let mut r = LineReader::default();
    let mut line = vec![b'x'; MAX_LINE];
    line.push(b'\n');
    assert_eq!(r.push(&line).unwrap().len(), 1);
    let mut r = LineReader::default();
    for _ in 0..MAX_LINE / CHUNK {
        r.push(&vec![b'x'; CHUNK]).unwrap();
    }
    // No newline yet, and already over: refused without waiting for the rest.
    assert_eq!(r.push(b"x"), Err(TooLong));
}

#[test]
fn the_version_is_checked_first() {
    assert_eq!(parse(br#"{"v": 2, "t": "hello"}"#).unwrap_err().code(), "version");
    assert_eq!(parse(br#"{"t": "hello"}"#).unwrap_err().code(), "bad_json");
    assert_eq!(parse(b"not json").unwrap_err().code(), "bad_json");
    assert_eq!(parse(b"[1]").unwrap_err().code(), "bad_json");
}

#[test]
fn a_known_message_with_a_field_missing_is_bad_json_and_an_unknown_one_is_kept() {
    assert_eq!(parse(br#"{"v": 1, "t": "key", "seq": 3}"#).unwrap_err().code(), "bad_json");
    let line = parse(br#"{"v": 1, "t": "teleport", "seq": 3, "ack": 2}"#).unwrap();
    assert_eq!(line, Line { seq: 3, ack: 2, msg: Incoming::Unknown("teleport".into()) });
}

#[test]
fn a_window_delta_carries_its_object_at_the_top_level() {
    let line = parse(br#"{"v": 1, "t": "window", "seq": 1, "ack": 0, "id": 5, "app": "Files", "title": "Home", "extra": true}"#).unwrap();
    let Incoming::Window(w) = line.msg else { panic!() };
    assert_eq!((w.id, w.frame, w.tileable, w.on_workspace), (5, None, true, true));
}

#[test]
fn every_line_out_is_stamped_and_ends_in_a_newline() {
    let l = out("pong", 7, json!({"re": 3}));
    assert!(l.ends_with('\n'));
    let v: Value = serde_json::from_str(&l).unwrap();
    assert_eq!(v, json!({"v": 1, "t": "pong", "seq": 7, "re": 3}));
}
