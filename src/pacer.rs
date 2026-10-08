use std::time::{Duration, Instant};

use crate::app::FramePacing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Poll,
    Wait,
    WaitUntil(Instant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Turn {
    pub(crate) redraw: bool,
    pub(crate) flow: Option<Flow>,
}

#[derive(Debug)]
pub(crate) struct Pacer {
    // Minimum gap between frames, or None for the legacy
    // spin-as-fast-as-possible loop. `None` is the default and
    // keeps `ControlFlow::Poll` + the self-sustaining
    // request_redraw() at the end of RedrawRequested; `Some(d)`
    // swaps both for a WaitUntil deadline driven from
    // about_to_wait. See `FramePacing`.
    interval: Option<Duration>,
    // `Reactive` pacing only: park in `ControlFlow::Wait` between
    // events instead of holding a deadline. Read together with
    // `animating` below.
    reactive: bool,
    // The last answer `RenderCallback::needs_frame` gave.
    //
    // ★ Starts `true` so the FIRST frame is never gated on an answer
    // nobody has asked for yet: a window that has drawn nothing has
    // nothing to keep showing, and parking before the first paint
    // would show an empty surface until the user happened to move
    // the mouse.
    animating: bool,
    // When the next frame is due. Meaningless (and never read)
    // while `interval` is None.
    next_frame_due: Instant,
    ring_due: Option<Instant>,
}

impl Pacer {
    pub(crate) fn new(pacing: FramePacing, now: Instant) -> Self {
        Self {
            interval: pacing.frame_interval(),
            reactive: matches!(pacing, FramePacing::Reactive(_)),
            animating: true,
            next_frame_due: now,
            ring_due: None,
        }
    }

    pub(crate) fn initial_flow(&self, now: Instant) -> Flow {
        match self.interval {
            None => Flow::Poll,
            Some(_) => Flow::WaitUntil(now),
        }
    }

    pub(crate) fn paced(&self) -> bool {
        self.interval.is_some()
    }

    pub(crate) fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }

    pub(crate) fn ring(&mut self, now: Instant, last_present: Instant) -> bool {
        let Some(interval) = self.interval else {
            return true;
        };
        let slot = last_present + interval;
        if now >= slot {
            self.ring_due = None;
            return true;
        }
        self.ring_due = Some(self.ring_due.map_or(slot, |due| due.min(slot)));
        false
    }

    pub(crate) fn redrawing(&mut self) {
        self.ring_due = None;
    }

    pub(crate) fn about_to_wait(&mut self, now: Instant) -> Turn {
        let Some(interval) = self.interval else {
            return Turn {
                redraw: false,
                flow: None,
            };
        };
        let rung = self.ring_due.is_some_and(|due| now >= due);
        if rung {
            self.ring_due = None;
        }
        // ── ★ NOTHING IN FLIGHT: SLEEP, DO NOT SCHEDULE ─────────────
        // `Reactive`'s whole value is this branch. With no animation
        // pending there is no next frame to be due, so the loop parks
        // in `ControlFlow::Wait` and the thread costs nothing at all
        // until a real event arrives. `next_frame_due` is resynced to
        // `now` on the way out, so waking from an arbitrarily long
        // park does not read as a stall and queue catch-up frames.
        if self.reactive && !self.animating {
            self.next_frame_due = now;
            return Turn {
                redraw: rung,
                flow: Some(self.ring_due.map_or(Flow::Wait, Flow::WaitUntil)),
            };
        }
        let mut redraw = rung;
        if now >= self.next_frame_due {
            // Advance from the previous DEADLINE, not from `now`, so
            // the cadence doesn't shed the render's own duration every
            // frame (that drift is how a 60 Hz cap silently becomes
            // 56 Hz). Resync to `now` only when a stall put us a whole
            // interval behind — a recovered stall must not queue a
            // burst of catch-up frames.
            let mut next = self.next_frame_due + interval;
            if next <= now {
                next = now + interval;
            }
            self.next_frame_due = next;
            redraw = true;
        }
        // Park until the next frame is due. Real input, resize and
        // IME events still wake the loop early — a deadline caps how
        // long we may SLEEP, it never delays an event.
        let wake = self
            .ring_due
            .map_or(self.next_frame_due, |due| due.min(self.next_frame_due));
        Turn {
            redraw,
            flow: Some(Flow::WaitUntil(wake)),
        }
    }
}

#[cfg(test)]
mod tests {
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
        last_present: Instant,
        redraws: usize,
        owed: bool,
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
            let turnstile = bell.connect(
                Box::new(move || {
                    let _ = tx.lock().unwrap().send(());
                }),
                Pacer::new(pacing, now),
            );
            let window = Self {
                turnstile,
                events,
                output: Arc::new(Mutex::new(Vec::new())),
                shown: Vec::new(),
                last_present: now
                    .checked_sub(Duration::from_secs(10))
                    .expect("the clock is ten seconds past its epoch"),
                redraws: 0,
                owed: false,
                racing: None,
            };
            (window, waker)
        }

        fn pacer(&mut self) -> &mut Pacer {
            &mut self.turnstile.pacer
        }

        fn redraw(&mut self, now: Instant) {
            self.redraws += 1;
            let fresh = Turnstile::redraw(self, ());
            let changed = !fresh.is_empty();
            self.shown.extend(fresh);
            self.pacer().set_animating(changed);
            if changed {
                self.last_present = now;
            }
        }

        fn turn(&mut self, now: Instant, wait: Duration) -> Option<Flow> {
            let mut redraw = std::mem::take(&mut self.owed);
            let wait = if redraw { Duration::ZERO } else { wait };
            if self.events.recv_timeout(wait).is_ok() {
                let last_present = self.last_present;
                redraw |= self.pacer().ring(now, last_present);
                while self.events.try_recv().is_ok() {
                    redraw |= self.pacer().ring(now, last_present);
                }
            }
            if redraw {
                self.redraw(now);
            }
            let turn = self.pacer().about_to_wait(now);
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
        window.pacer().set_animating(false);
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
        window.pacer().set_animating(false);
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
        let presented = window.last_present;
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
        pacer.set_animating(false);
        let last_present = start;
        assert!(!pacer.ring(start + Duration::from_millis(1), last_present));
        let due = last_present + Duration::from_secs_f64(1.0 / 60.0);
        assert_eq!(
            pacer.about_to_wait(start + Duration::from_millis(2)),
            Turn {
                redraw: false,
                flow: Some(Flow::WaitUntil(due))
            }
        );
        assert_eq!(
            pacer.about_to_wait(due),
            Turn {
                redraw: true,
                flow: Some(Flow::Wait)
            }
        );
    }

    #[test]
    fn a_continuous_window_redraws_on_every_ring_and_never_sets_a_flow() {
        let start = Instant::now();
        let mut pacer = Pacer::new(FramePacing::Continuous, start);
        assert!(pacer.ring(start, start));
        assert_eq!(pacer.initial_flow(start), Flow::Poll);
        assert_eq!(
            pacer.about_to_wait(start),
            Turn {
                redraw: false,
                flow: None
            }
        );
    }

    #[test]
    fn a_redraw_for_any_reason_settles_a_deferred_ring() {
        let start = Instant::now();
        let mut pacer = Pacer::new(FramePacing::Reactive(SIXTY), start);
        pacer.set_animating(false);
        assert!(!pacer.ring(start, start));
        pacer.redrawing();
        assert_eq!(
            pacer.about_to_wait(start + Duration::from_secs(1)),
            Turn {
                redraw: false,
                flow: Some(Flow::Wait)
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
                flow: Some(Flow::WaitUntil(start + interval))
            }
        );
        let early = pacer.about_to_wait(start + interval / 2);
        assert_eq!(
            early,
            Turn {
                redraw: false,
                flow: Some(Flow::WaitUntil(start + interval))
            }
        );
        let stalled = start + interval * 5;
        assert_eq!(
            pacer.about_to_wait(stalled),
            Turn {
                redraw: true,
                flow: Some(Flow::WaitUntil(stalled + interval))
            }
        );
    }
}
