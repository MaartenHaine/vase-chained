//! The mirror: optimistic at once, never undone by a delta that predates the effect, and equal to
//! what the extension reports once it has caught up (protocol §6.1).

use super::*;
use crate::protocol::WireWindow;

fn win(id: u64, frame: WireRect) -> WireWindow {
    WireWindow {
        id,
        pid: id as i32,
        app: format!("app{id}"),
        app_id: String::new(),
        title: format!("w{id}"),
        frame: Some(frame),
        layer: 0,
        tileable: true,
        minimized: false,
        fullscreen: false,
        on_workspace: true,
        x11: false,
    }
}

fn display() -> WireDisplay {
    WireDisplay { id: 1, connector: "HDMI-1".into(), bounds: [0.0, 0.0, 1000.0, 800.0], work_area: [0.0, 0.0, 1000.0, 800.0], scale: 1.0 }
}

fn mirror(n: u64) -> Mirror {
    let mut m = Mirror::default();
    m.load(vec![display()], 0, (1..=n).map(|i| win(i, [0.0, 0.0, 100.0, 100.0])).collect(), Some(1), Vec::new());
    m
}

fn front(m: &Mirror) -> u64 {
    m.stack()[0].id
}

#[test]
fn a_raise_shows_at_once_and_survives_a_delta_sent_before_it_was_applied() {
    let mut m = mirror(3);
    m.raise(3);
    assert_eq!(front(&m), 3, "the read after the raise, in the same dispatch, already sees it");
    m.seal(7);
    // The extension restacked for its own reasons before it saw batch 7.
    m.stacking(vec![2, 1, 3], 6);
    assert_eq!(front(&m), 3);
    // Once it reports having applied batch 7, its report is the truth.
    m.stacking(vec![1, 3, 2], 7);
    assert_eq!(front(&m), 1);
    assert_eq!(m.pending(), 0);
}

#[test]
fn a_placement_lasts_until_it_settles() {
    let mut m = mirror(2);
    let target = Rect::new(500.0, 0.0, 500.0, 800.0);
    m.set_frame(2, target);
    m.seal(4);
    assert!(m.placing(2));
    assert_eq!(m.get(2).unwrap().frame, Some([500.0, 0.0, 500.0, 800.0]));
    // Applied (ack 4), still in flight: the delta leaves the frame out, and the placement stands.
    let mut moving = win(2, [0.0, 0.0, 0.0, 0.0]);
    moving.frame = None;
    moving.title = "renamed".into();
    m.window(moving, 4);
    assert_eq!(m.get(2).unwrap().frame, Some([500.0, 0.0, 500.0, 800.0]));
    assert_eq!(m.get(2).unwrap().title, "renamed");
    // A frame from before the batch doesn't override it either.
    m.window(win(2, [0.0, 0.0, 100.0, 100.0]), 3);
    assert_eq!(m.get(2).unwrap().frame, Some([500.0, 0.0, 500.0, 800.0]));
    m.settled(2, 4, [500.0, 0.0, 498.0, 800.0], 4);
    assert!(!m.placing(2));
    assert_eq!(m.get(2).unwrap().frame, Some([500.0, 0.0, 498.0, 800.0]), "the achieved frame, not the request");
}

#[test]
fn a_new_window_goes_in_front_and_a_gone_one_takes_its_ops_with_it() {
    let mut m = mirror(2);
    m.window(win(9, [0.0, 0.0, 300.0, 300.0]), 0);
    assert_eq!(front(&m), 9);
    m.raise(9);
    m.set_frame(9, Rect::new(1.0, 2.0, 3.0, 4.0));
    m.seal(1);
    m.window_gone(9, 0);
    assert_eq!(m.pending(), 0);
    assert!(m.get(9).is_none());
}

#[test]
fn ops_never_sent_can_be_dropped() {
    let mut m = mirror(2);
    m.raise(2);
    m.drop_unsent();
    assert_eq!(front(&m), 1);
}

/// A small deterministic generator, so the property test needs no crate and reruns the same cases.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The extension and its compositor, as far as the mirror can see them.
struct World {
    frames: HashMap<u64, WireRect>,
    /// Frontmost first.
    stack: Vec<u64>,
    focus: Option<u64>,
    /// Applied placements still settling: (window, batch, target).
    settling: Vec<(u64, u64, WireRect)>,
    ack: u64,
}

/// A delta on its way to the daemon: deltas keep their order, but arrive late.
enum Delta {
    Stacking(Vec<u64>, u64),
    Focus(Option<u64>, u64),
    Window(u64, Option<WireRect>, u64),
    Settled(u64, u64, WireRect, u64),
}

enum Eff {
    Frame(u64, WireRect),
    Raise(u64),
    Focus(u64),
}

impl World {
    fn front(&mut self, id: u64) {
        self.stack.retain(|w| *w != id);
        self.stack.insert(0, id);
    }

    fn apply(&mut self, batch: u64, effects: &[Eff], out: &mut VecDeque<Delta>) {
        for e in effects {
            match e {
                Eff::Frame(id, r) => {
                    // One request in flight per window: a newer one replaces the held rect.
                    self.settling.retain(|(w, _, _)| w != id);
                    self.settling.push((*id, batch, *r));
                }
                Eff::Raise(id) => self.front(*id),
                Eff::Focus(id) => {
                    self.front(*id);
                    self.focus = Some(*id);
                }
            }
        }
        self.ack = batch;
        out.push_back(Delta::Stacking(self.stack.clone(), self.ack));
        out.push_back(Delta::Focus(self.focus, self.ack));
    }
}

use std::collections::VecDeque;

#[test]
fn the_mirror_converges_after_random_effects_and_late_deltas() {
    for case in 0..300u64 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (case + 1).wrapping_mul(0x2545_F491_4F6C_DD1D));
        let n = 2 + rng.below(5);
        let mut m = mirror(n);
        let mut world = World { frames: (1..=n).map(|i| (i, [0.0, 0.0, 100.0, 100.0])).collect(), stack: (1..=n).collect(), focus: Some(1), settling: Vec::new(), ack: 0 };
        let mut batches: VecDeque<(u64, Vec<Eff>)> = VecDeque::new();
        let mut deltas: VecDeque<Delta> = VecDeque::new();
        let mut seq = 0u64;
        let mut newest_raise: Option<(u64, u64)> = None;
        for _ in 0..60 {
            match rng.below(6) {
                // The daemon emits a batch.
                0 | 1 => {
                    seq += 1;
                    let mut effects = Vec::new();
                    for _ in 0..1 + rng.below(3) {
                        let id = 1 + rng.below(n);
                        match rng.below(3) {
                            0 => {
                                let r = [rng.below(4) as f64 * 100.0, 0.0, 100.0 + rng.below(4) as f64 * 50.0, 300.0];
                                m.set_frame(id, rect(r));
                                assert_eq!(m.get(id).unwrap().frame, Some(r), "case {case}: a placement shows at once");
                                effects.push(Eff::Frame(id, r));
                            }
                            1 => {
                                m.raise(id);
                                assert_eq!(front(&m), id, "case {case}: a raise shows at once");
                                newest_raise = Some((seq, id));
                                effects.push(Eff::Raise(id));
                            }
                            _ => {
                                m.set_focus(id);
                                assert_eq!(m.focused(), Some(id));
                                newest_raise = Some((seq, id));
                                effects.push(Eff::Focus(id));
                            }
                        }
                    }
                    m.seal(seq);
                    batches.push_back((seq, effects));
                }
                // The extension applies the oldest batch.
                2 => {
                    if let Some((batch, effects)) = batches.pop_front() {
                        world.apply(batch, &effects, &mut deltas);
                    }
                }
                // A placement settles, a hair short (a cell-sized terminal).
                3 => {
                    if !world.settling.is_empty() {
                        let (id, batch, r) = world.settling.remove(rng.below(world.settling.len() as u64) as usize);
                        let got = [r[0], r[1], r[2] - rng.below(2) as f64, r[3]];
                        world.frames.insert(id, got);
                        deltas.push_back(Delta::Settled(id, batch, got, world.ack));
                    }
                }
                // The user clicks a window, or a title changes.
                4 => {
                    let id = 1 + rng.below(n);
                    if rng.below(2) == 0 {
                        world.front(id);
                        world.focus = Some(id);
                        deltas.push_back(Delta::Stacking(world.stack.clone(), world.ack));
                        deltas.push_back(Delta::Focus(world.focus, world.ack));
                    } else {
                        let frame = (!world.settling.iter().any(|(w, _, _)| *w == id)).then(|| world.frames[&id]);
                        deltas.push_back(Delta::Window(id, frame, world.ack));
                    }
                }
                // A delta reaches the daemon.
                _ => {
                    if let Some(d) = deltas.pop_front() {
                        let ack = deliver(&mut m, d);
                        if let Some((batch, id)) = newest_raise {
                            if ack < batch {
                                assert_eq!(front(&m), id, "case {case}: a delta older than the raise undid it");
                            }
                        }
                    }
                }
            }
        }
        // Quiesce: every batch applied, every placement settled, every delta delivered.
        while let Some((batch, effects)) = batches.pop_front() {
            world.apply(batch, &effects, &mut deltas);
        }
        for (id, batch, r) in std::mem::take(&mut world.settling) {
            world.frames.insert(id, r);
            deltas.push_back(Delta::Settled(id, batch, r, world.ack));
        }
        deltas.push_back(Delta::Stacking(world.stack.clone(), world.ack));
        while let Some(d) = deltas.pop_front() {
            deliver(&mut m, d);
        }
        assert_eq!(m.pending(), 0, "case {case}: ops left over");
        assert_eq!(m.stack().iter().map(|w| w.id).collect::<Vec<_>>(), world.stack, "case {case}: stacking");
        assert_eq!(m.focused(), world.focus, "case {case}: focus");
        for (id, frame) in &world.frames {
            assert_eq!(m.get(*id).unwrap().frame, Some(*frame), "case {case}: frame of {id}");
        }
    }
}

fn deliver(m: &mut Mirror, d: Delta) -> u64 {
    match d {
        Delta::Stacking(ids, ack) => {
            m.stacking(ids, ack);
            ack
        }
        Delta::Focus(id, ack) => {
            m.focus(id, ack);
            ack
        }
        Delta::Window(id, frame, ack) => {
            let mut w = win(id, [0.0; 4]);
            w.frame = frame;
            m.window(w, ack);
            ack
        }
        Delta::Settled(id, re, frame, ack) => {
            m.settled(id, re, frame, ack);
            ack
        }
    }
}

#[test]
fn an_x11_window_a_few_pixels_short_counts_as_placed_and_a_wayland_one_does_not() {
    let mut m = Mirror::default();
    let mut xterm = win(1, [0.0, 0.0, 997.0, 1043.0]);
    xterm.x11 = true;
    let wayland = win(2, [0.0, 0.0, 997.0, 1043.0]);
    m.load(vec![display()], 0, vec![xterm, wayland], Some(1), Vec::new());
    let want = Rect::new(0.0, 0.0, 1000.0, 1048.0);
    assert!(m.placed_as(1, want), "short by less than one resize increment (§6.2)");
    assert!(!m.placed_as(2, want), "exact on Wayland");
    assert!(m.placed_as(2, Rect::new(0.0, 0.0, 997.0, 1043.0)));
    // Never larger, never moved, never short by a whole increment.
    assert!(!m.placed_as(1, Rect::new(0.0, 0.0, 990.0, 1048.0)));
    assert!(!m.placed_as(1, Rect::new(4.0, 0.0, 1000.0, 1048.0)));
    assert!(!m.placed_as(1, Rect::new(0.0, 0.0, 997.0 + X11_MAX_INC.0, 1048.0)));
    assert!(!m.placed_as(3, want), "an unknown window is never placed");
}
