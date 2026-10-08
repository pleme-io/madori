use super::*;
use crate::doorbell::{Doorbell, Drains, Turnstile};
use std::num::NonZeroU32;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::task::Waker;

const SIXTY: NonZeroU32 = NonZeroU32::new(60).unwrap();

struct Window {
    turnstile: Turnstile,
    events: mpsc::Receiver<()>,
    output: Arc<Mutex<Vec<u8>>>,
    shown: Vec<u8>,
    redraws: usize,
    owed: bool,
    withholding: bool,
    racing: Option<(Waker, u8)>,
}

impl Drains<()> for Window {
    type Drained = Vec<u8>;

    fn turnstile(&mut self) -> &mut Turnstile {
        &mut self.turnstile
    }

    fn drain(&mut self, (): ()) -> Vec<u8> {
        let fresh = std::mem::take(&mut *self.output.lock().unwrap());
        if let Some((waker, byte)) = self.racing.take() {
            self.output.lock().unwrap().push(byte);
            waker.wake();
        }
        fresh
    }
}

impl Window {
    fn open(pacing: FramePacing, now: Instant) -> (Self, Waker) {
        let bell = Doorbell::new();
        let waker = bell.waker();
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let mut pacer = Pacer::new(pacing, now);
        pacer.last_present = now
            .checked_sub(Duration::from_secs(10))
            .expect("the clock is ten seconds past its epoch");
        pacer.debts.settle();
        let turnstile = bell.connect(
            Box::new(move || {
                let _ = tx.lock().unwrap().send(());
            }),
            pacer,
        );
        let window = Self {
            turnstile,
            events,
            output: Arc::new(Mutex::new(Vec::new())),
            shown: Vec::new(),
            redraws: 0,
            owed: false,
            withholding: false,
            racing: None,
        };
        (window, waker)
    }

    fn pacer(&mut self) -> &mut Pacer {
        &mut self.turnstile.pacer
    }

    fn park(&mut self, now: Instant) {
        for _ in 0..IDLE_TICKS {
            assert!(self.pacer().decide(now, FrameDemand::Idle).is_none());
        }
        assert_eq!(self.pacer().heat(), Heat::Parked);
    }

    fn redraw(&mut self, now: Instant) {
        self.redraws += 1;
        let fresh = Turnstile::redraw(self, ());
        let demand = if fresh.is_empty() {
            FrameDemand::Idle
        } else {
            FrameDemand::Now
        };
        self.shown.extend(fresh);
        if self.pacer().decide(now, demand).is_some() {
            self.pacer().presented(now);
        }
    }

    fn rang(&mut self, now: Instant) -> bool {
        match self.pacer().ring(now) {
            Wake::Redraw => true,
            Wake::Defer => false,
            Wake::Drain => {
                let fresh = Turnstile::redraw(self, ());
                self.shown.extend(fresh);
                false
            }
        }
    }

    fn turn(&mut self, now: Instant, wait: Duration) -> Option<Flow> {
        let mut redraw = std::mem::take(&mut self.owed);
        let wait = if redraw { Duration::ZERO } else { wait };
        if self.events.recv_timeout(wait).is_ok() {
            redraw |= self.rang(now);
            while self.events.try_recv().is_ok() {
                redraw |= self.rang(now);
            }
        }
        if redraw {
            self.pacer().requested(now);
            if !self.withholding {
                self.pacer().arrived();
                self.redraw(now);
            }
        }
        let (turn, drained) = Turnstile::wait(self, now, ());
        self.shown.extend(drained.into_iter().flatten());
        self.owed = turn.redraw;
        turn.flow
    }

    fn settle(&mut self, now: Instant) -> Option<Flow> {
        let mut flow = self.turn(now, Duration::ZERO);
        while self.owed {
            flow = self.turn(now, Duration::ZERO);
        }
        flow
    }

    fn print(&self, waker: &Waker, byte: u8) {
        let output = Arc::clone(&self.output);
        let waker = waker.clone();
        std::thread::spawn(move || {
            output.lock().unwrap().push(byte);
            waker.wake();
        })
        .join()
        .unwrap();
    }
}

fn parked(pacing: FramePacing, waker_of: impl Fn(&Waker) -> Waker) {
    let start = Instant::now();
    let (mut window, bell) = Window::open(pacing, start);
    window.park(start);
    assert_eq!(
        window.turn(start, Duration::ZERO),
        Some(Flow::Wait),
        "an idle Reactive window parks"
    );
    window.print(&waker_of(&bell), b'x');
    let before = window.redraws;
    let flow = window.turn(start + Duration::from_secs(1), Duration::from_secs(1));
    assert_eq!(
        window.redraws - before,
        1,
        "the ring's own turn redraws, before the loop waits again"
    );
    assert_eq!(window.shown, b"x");
    assert!(matches!(flow, Some(Flow::WaitUntil(_))));
}

#[test]
fn a_ring_while_parked_yields_a_redraw_within_one_loop_turn() {
    parked(FramePacing::Reactive(SIXTY), Waker::clone);
}

#[test]
#[should_panic(expected = "the ring's own turn redraws")]
fn with_the_wake_off_a_parked_window_never_shows_its_output() {
    parked(FramePacing::Reactive(SIXTY), |_| Waker::noop().clone());
}

#[test]
fn a_ring_that_lands_while_the_loop_drains_is_answered_by_the_next_turn() {
    let start = Instant::now();
    let (mut window, bell) = Window::open(FramePacing::Reactive(SIXTY), start);
    window.park(start);
    assert_eq!(window.turn(start, Duration::ZERO), Some(Flow::Wait));
    window.racing = Some((bell.clone(), b'z'));
    bell.wake_by_ref();
    let flow = window.turn(start + Duration::from_secs(1), Duration::from_millis(200));
    assert_eq!(window.redraws, 1, "the empty ring redrew once");
    assert!(window.shown.is_empty(), "its drain read nothing");
    assert_eq!(flow, Some(Flow::Wait), "and the window parked again");
    window.turn(start + Duration::from_secs(2), Duration::from_millis(200));
    assert_eq!(
        window.shown, b"z",
        "the item queued behind the drain's read was stranded: its ring found the flag still raised"
    );
}

#[test]
fn a_capped_window_shows_an_isolated_ring_at_once_and_a_ring_inside_its_interval_by_its_slot() {
    let start = Instant::now();
    let (mut window, bell) = Window::open(FramePacing::Capped(SIXTY), start);
    let interval = Duration::from_secs_f64(1.0 / 60.0);
    let now = start + Duration::from_millis(3);
    window.print(&bell, b'a');
    let before = window.redraws;
    window.turn(now, Duration::from_secs(1));
    assert_eq!(
        window.redraws - before,
        1,
        "an isolated ring redraws in its own turn"
    );
    assert_eq!(window.shown, b"a");
    window.settle(now);
    let presented = window.pacer().last_present;
    window.print(&bell, b'b');
    let before = window.redraws;
    let flow = window.turn(presented + Duration::from_millis(2), Duration::from_secs(1));
    assert_eq!(window.redraws, before, "inside the interval the ring waits");
    let Some(Flow::WaitUntil(wake)) = flow else {
        panic!("a deferred ring must hold a deadline, got {flow:?}");
    };
    assert!(
        wake <= presented + interval,
        "the ring waits no longer than its slot"
    );
    window.settle(wake);
    assert_eq!(window.shown, b"ab", "the wake shows it");
}

#[test]
fn a_reactive_window_holds_a_deferred_ring_as_a_deadline_not_a_park() {
    let start = Instant::now();
    let mut pacer = Pacer::new(FramePacing::Reactive(SIXTY), start);
    pacer.debts.settle();
    for _ in 0..IDLE_TICKS {
        let _ = pacer.decide(start, FrameDemand::Idle);
    }
    let last_present = start;
    assert_eq!(pacer.ring(start + Duration::from_millis(1)), Wake::Defer);
    let due = last_present + Duration::from_secs_f64(1.0 / 60.0);
    assert_eq!(
        pacer.about_to_wait(start + Duration::from_millis(2)),
        Turn {
            redraw: false,
            drain: false,
            flow: Some(Flow::WaitUntil(due)),
        }
    );
    assert_eq!(
        pacer.about_to_wait(due),
        Turn {
            redraw: true,
            drain: false,
            flow: Some(Flow::Wait),
        }
    );
}

#[test]
fn a_continuous_window_redraws_on_every_ring_and_never_sets_a_flow() {
    let start = Instant::now();
    let mut pacer = Pacer::new(FramePacing::Continuous, start);
    assert_eq!(pacer.ring(start), Wake::Redraw);
    assert_eq!(pacer.initial_flow(start), Flow::Poll);
    assert_eq!(
        pacer.about_to_wait(start),
        Turn {
            redraw: false,
            drain: false,
            flow: None,
        }
    );
}

#[test]
fn a_redraw_for_any_reason_settles_a_deferred_ring() {
    let start = Instant::now();
    let mut pacer = Pacer::new(FramePacing::Reactive(SIXTY), start);
    pacer.debts.settle();
    for _ in 0..IDLE_TICKS {
        let _ = pacer.decide(start, FrameDemand::Idle);
    }
    assert_eq!(pacer.ring(start), Wake::Defer);
    pacer.redrawing();
    assert_eq!(
        pacer.about_to_wait(start + Duration::from_secs(1)),
        Turn {
            redraw: false,
            drain: false,
            flow: Some(Flow::Wait),
        }
    );
}

#[test]
fn the_capped_cadence_is_unchanged_by_the_ring_machinery() {
    let start = Instant::now();
    let mut pacer = Pacer::new(FramePacing::Capped(SIXTY), start);
    let interval = Duration::from_secs_f64(1.0 / 60.0);
    let first = pacer.about_to_wait(start);
    assert_eq!(
        first,
        Turn {
            redraw: true,
            drain: false,
            flow: Some(Flow::WaitUntil(start + interval)),
        }
    );
    let early = pacer.about_to_wait(start + interval / 2);
    assert_eq!(
        early,
        Turn {
            redraw: false,
            drain: false,
            flow: Some(Flow::WaitUntil(start + interval)),
        }
    );
    let stalled = start + interval * 5;
    assert_eq!(
        pacer.about_to_wait(stalled),
        Turn {
            redraw: true,
            drain: false,
            flow: Some(Flow::WaitUntil(stalled + interval)),
        }
    );
}

#[test]
fn the_display_rate_bounds_a_reactive_cadence_and_never_raises_its_ceiling() {
    let start = Instant::now();
    let mut pacer = Pacer::new(FramePacing::Reactive(NonZeroU32::MAX), start);
    assert_eq!(
        pacer.interval(),
        Some(Duration::from_secs_f64(1.0 / 60.0)),
        "until the display is read an unbounded demand window ticks at 60 Hz, never at 0 ns"
    );
    pacer.set_display_millihertz(Some(120_000));
    assert_eq!(pacer.interval(), Some(Duration::from_secs_f64(1.0 / 120.0)));
    let mut capped60 = Pacer::new(FramePacing::Reactive(SIXTY), start);
    capped60.set_display_millihertz(Some(120_000));
    assert_eq!(
        capped60.interval(),
        FramePacing::Reactive(SIXTY).frame_interval()
    );
    capped60.set_display_millihertz(None);
    assert_eq!(
        capped60.interval(),
        FramePacing::Reactive(SIXTY).frame_interval()
    );
    let mut capped = Pacer::new(FramePacing::Capped(SIXTY), start);
    capped.set_display_millihertz(Some(120_000));
    assert_eq!(
        capped.interval(),
        FramePacing::Capped(SIXTY).frame_interval(),
        "Capped keeps today's rate whatever the display"
    );
}

fn hidden_while_a_ring_is_unanswered(hide: impl FnOnce(&mut Window, &Waker, Instant) -> Instant) {
    let start = Instant::now();
    let (mut window, bell) = Window::open(FramePacing::Reactive(SIXTY), start);
    window.print(&bell, b'a');
    window.turn(start, Duration::from_secs(1));
    window.settle(start);
    assert_eq!(window.shown, b"a");
    let hidden_at = hide(&mut window, &bell, start);
    assert!(window.pacer().hidden(), "the window is hidden");
    window.print(&bell, b'c');
    window.print(&bell, b'd');
    let sent = window.events.try_iter().count();
    assert!(
        sent >= 1,
        "a ring reaches the loop once the window is hidden"
    );
    for _ in 0..sent {
        assert!(
            !window.rang(hidden_at),
            "a hidden window never asks for a frame"
        );
    }
    assert_eq!(
        window.shown, b"abcd",
        "a hidden window drains what rings it, not only once it is shown again"
    );
}

#[test]
fn on_wayland_a_window_hidden_by_a_withheld_redraw_still_drains_what_rings_it() {
    hidden_while_a_ring_is_unanswered(|window, bell, start| {
        window.pacer().infer_withheld_redraws(true);
        window.withholding = true;
        let rung = start + Duration::from_millis(100);
        window.print(bell, b'b');
        window.turn(rung, Duration::from_secs(1));
        assert!(
            !window.pacer().hidden(),
            "one withheld redraw is not yet a hidden window"
        );
        let inferred = rung + Duration::from_millis(100);
        window.turn(inferred, Duration::ZERO);
        inferred
    });
}

#[test]
fn a_window_occluded_while_its_ring_waits_for_the_slot_still_drains_what_rings_it() {
    hidden_while_a_ring_is_unanswered(|window, bell, start| {
        window.print(bell, b'b');
        let early = start + Duration::from_millis(2);
        let redraws = window.redraws;
        window.turn(early, Duration::from_secs(1));
        assert_eq!(
            window.redraws, redraws,
            "inside the interval the ring waits for its slot"
        );
        let shift = window.pacer().set_occluded(true);
        assert_eq!(shift, Shift::Hid);
        let drained = Turnstile::shift(window, shift, ());
        window.shown.extend(drained.into_iter().flatten());
        early
    });
}
