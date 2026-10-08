use std::cell::Cell;
use std::convert::Infallible;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::Waker;
use std::time::{Duration, Instant};

use super::{Flow, Heat, Pacer, Shift, Surface, Visible, Wake};
use crate::app::{FrameDebt, FramePacing, reask_after};
use crate::doorbell::{Doorbell, Drains, Turnstile};
use crate::event::{AppEvent, MouseEvent};
use crate::render::FrameDemand;

const DISPLAY_MHZ: u32 = 120_000;
const SIXTY: NonZeroU32 = NonZeroU32::new(60).unwrap();

struct Fake {
    acquires: Cell<u32>,
}

impl Surface for Fake {
    type Frame = ();
    type Error = Infallible;

    fn acquire(&self, visible: Visible) -> Result<(), Infallible> {
        let Visible { sealed: () } = visible;
        self.acquires.set(self.acquires.get() + 1);
        Ok(())
    }
}

trait Scene {
    fn demand(&mut self, now: Instant) -> FrameDemand;
    fn drew(&mut self, _now: Instant) {}
    fn drain(&mut self) {}
}

struct Quiet;

impl Scene for Quiet {
    fn demand(&mut self, _now: Instant) -> FrameDemand {
        FrameDemand::Idle
    }
}

#[derive(Default)]
struct Output {
    queued: u32,
    dirty: bool,
}

impl Scene for Output {
    fn demand(&mut self, _now: Instant) -> FrameDemand {
        if self.dirty {
            FrameDemand::Now
        } else {
            FrameDemand::Idle
        }
    }

    fn drew(&mut self, _now: Instant) {
        self.dirty = false;
    }

    fn drain(&mut self) {
        if self.queued > 0 {
            self.queued = 0;
            self.dirty = true;
        }
    }
}

struct Blink {
    epoch: Instant,
    half: Duration,
    drawn: Option<u128>,
}

impl Blink {
    fn phase(&self, now: Instant) -> u128 {
        now.duration_since(self.epoch).as_nanos() / self.half.as_nanos()
    }
}

impl Scene for Blink {
    fn demand(&mut self, now: Instant) -> FrameDemand {
        let phase = self.phase(now);
        if self.drawn != Some(phase) {
            return FrameDemand::Now;
        }
        let next = u32::try_from(phase + 1).expect("a short run");
        FrameDemand::At(self.epoch + self.half * next)
    }

    fn drew(&mut self, now: Instant) {
        self.drawn = Some(self.phase(now));
    }
}

struct Glide {
    until: Option<Instant>,
}

impl Scene for Glide {
    fn demand(&mut self, now: Instant) -> FrameDemand {
        match self.until {
            Some(until) if now < until => FrameDemand::Continuous,
            _ => FrameDemand::Idle,
        }
    }
}

struct Wake2 {
    at: Instant,
    seen: bool,
}

impl Scene for Wake2 {
    fn demand(&mut self, now: Instant) -> FrameDemand {
        if self.seen {
            FrameDemand::Idle
        } else if now >= self.at {
            FrameDemand::Now
        } else {
            FrameDemand::At(self.at)
        }
    }

    fn drew(&mut self, now: Instant) {
        if now >= self.at {
            self.seen = true;
        }
    }
}

struct Rig<S: Scene> {
    turnstile: Turnstile,
    bell: Waker,
    sent: Arc<AtomicU32>,
    surface: Fake,
    scene: S,
    start: Instant,
    now: Instant,
    pending: bool,
    withholding: bool,
    turns: u32,
    drains: u32,
    presents: Vec<Instant>,
}

impl<S: Scene> Drains<()> for Rig<S> {
    type Drained = ();

    fn turnstile(&mut self) -> &mut Turnstile {
        &mut self.turnstile
    }

    fn drain(&mut self, (): ()) {
        self.drains += 1;
        self.scene.drain();
    }
}

impl<S: Scene> Rig<S> {
    fn open(pacing: FramePacing, scene: S) -> Self {
        Self::open_on(pacing, scene, false)
    }

    fn open_on(pacing: FramePacing, scene: S, wayland: bool) -> Self {
        let start = Instant::now();
        let mut pacer = Pacer::new(pacing, start);
        pacer.set_display_millihertz(Some(DISPLAY_MHZ));
        pacer.infer_withheld_redraws(wayland);
        let doorbell = Doorbell::new();
        let bell = doorbell.waker();
        let sent = Arc::new(AtomicU32::new(0));
        let line = Arc::clone(&sent);
        let turnstile = doorbell.connect(
            Box::new(move || {
                line.fetch_add(1, Ordering::SeqCst);
            }),
            pacer,
        );
        let mut rig = Self {
            turnstile,
            bell,
            sent,
            surface: Fake {
                acquires: Cell::new(0),
            },
            scene,
            start,
            now: start,
            pending: false,
            withholding: false,
            turns: 0,
            drains: 0,
            presents: Vec::new(),
        };
        rig.request();
        rig.settle();
        rig
    }

    fn interval(&self) -> Duration {
        self.turnstile
            .pacer
            .interval()
            .unwrap_or(Duration::from_millis(1))
    }

    fn at(&self, offset: Duration) -> Instant {
        self.start + offset
    }

    fn request(&mut self) {
        self.pending = true;
        self.turnstile.pacer.requested(self.now);
    }

    fn deliver(&mut self) {
        self.pending = false;
        self.turnstile.pacer.arrived();
        Turnstile::redraw(self, ());
        self.turns += 1;
        let demand = self.scene.demand(self.now);
        if let Some(visible) = self.turnstile.pacer.decide(self.now, demand) {
            self.surface.acquire(visible).unwrap_or_else(|e| match e {});
            self.turnstile.pacer.presented(self.now);
            self.scene.drew(self.now);
            self.presents.push(self.now);
        }
        if !self.turnstile.pacer.paced() {
            self.request();
        }
    }

    fn deliverable(&self) -> bool {
        self.pending && !self.withholding
    }

    fn settle(&mut self) {
        let now = self.now;
        self.run_until(now);
    }

    fn answer_rings(&mut self) {
        while self
            .sent
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            let now = self.now;
            match self.turnstile.pacer.ring(now) {
                Wake::Redraw => self.request(),
                Wake::Defer => {}
                Wake::Drain => Turnstile::redraw(self, ()),
            }
        }
    }

    fn run_until(&mut self, until: Instant) {
        let mut spins = 0u32;
        loop {
            spins += 1;
            assert!(spins < 2_000_000, "the loop never waits");
            self.answer_rings();
            if self.deliverable() {
                self.deliver();
            }
            let now = self.now;
            let (turn, _) = Turnstile::wait(self, now, ());
            if turn.redraw {
                self.request();
            }
            let at = match turn.flow {
                Some(Flow::Poll) | None => {
                    if self.now >= until {
                        break;
                    }
                    self.now = (self.now + Duration::from_millis(1)).min(until);
                    continue;
                }
                Some(Flow::Wait) => None,
                Some(Flow::WaitUntil(at)) => Some(at),
            };
            if self.deliverable() || self.sent.load(Ordering::SeqCst) > 0 {
                continue;
            }
            match at {
                Some(at) if at <= until => {
                    assert!(at > self.now, "a deadline in the past with nothing to do");
                    self.now = at;
                }
                _ => break,
            }
        }
        self.now = self.now.max(until);
    }

    fn ring(&mut self) {
        self.bell.wake_by_ref();
        self.settle();
    }

    fn event(&mut self, event: &AppEvent) {
        if reask_after(event) && self.turnstile.pacer.reask(self.now) {
            self.request();
        }
        self.settle();
    }

    fn motion(&mut self) {
        self.event(&AppEvent::Mouse(MouseEvent::Moved { x: 1.0, y: 1.0 }));
    }

    fn shift(&mut self, shift: Shift) {
        if shift == Shift::Shown {
            self.request();
        }
        Turnstile::shift(self, shift, ());
        self.settle();
    }

    fn occlude(&mut self, occluded: bool) {
        let shift = self.turnstile.pacer.set_occluded(occluded);
        self.shift(shift);
    }

    fn minimize(&mut self, minimized: bool) {
        let shift = self.turnstile.pacer.set_minimized(minimized);
        self.shift(shift);
    }

    fn resize(&mut self) {
        self.turnstile.pacer.owe(FrameDebt::Resized);
        self.request();
        self.settle();
    }

    fn acquires(&self) -> u32 {
        self.surface.acquires.get()
    }

    fn presents_after(&self, from: Instant) -> usize {
        self.presents.iter().filter(|p| **p >= from).count()
    }
}

impl Rig<Output> {
    fn print(&mut self) {
        self.scene.queued += 1;
        self.ring();
    }
}

fn pacings() -> [FramePacing; 3] {
    [
        FramePacing::Reactive(NonZeroU32::MAX),
        FramePacing::Capped(SIXTY),
        FramePacing::Continuous,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expect {
    parks: bool,
    hides: bool,
}

const fn expect(pacing: FramePacing) -> Expect {
    match pacing {
        FramePacing::Reactive(_) => Expect {
            parks: true,
            hides: true,
        },
        FramePacing::Capped(_) | FramePacing::Continuous => Expect {
            parks: false,
            hides: false,
        },
    }
}

fn idle_turns(pacing: FramePacing) -> u32 {
    let mut rig = Rig::open(pacing, Quiet);
    let from = rig.turns;
    let end = rig.at(Duration::from_secs(60));
    rig.run_until(end);
    rig.turns - from
}

#[test]
fn quiet_for_sixty_seconds_a_demand_window_ticks_at_most_once_a_second_and_the_kept_pacings_do_not()
{
    for pacing in pacings() {
        let turns = idle_turns(pacing);
        if expect(pacing).parks {
            assert!(
                turns <= 60,
                "{pacing:?}: {turns} loop turns over 60 quiet seconds"
            );
            assert!(
                turns <= 2,
                "{pacing:?}: after its first frame a quiet window takes its two idle ticks and parks, not {turns}"
            );
        } else {
            assert!(
                turns > 60,
                "{pacing:?} is a negative control: it keeps ticking ({turns} turns in 60 s)"
            );
        }
    }
}

#[test]
fn a_parked_window_draws_the_first_frame_after_idle_at_once() {
    for pacing in pacings() {
        let mut rig = Rig::open(pacing, Output::default());
        rig.run_until(rig.at(Duration::from_secs(2)));
        let ring_at = rig.now;
        rig.print();
        assert_eq!(
            rig.presents.last().copied(),
            Some(ring_at),
            "{pacing:?}: the first frame after idle is drawn in the ring's own turn"
        );
    }
}

#[test]
fn a_hot_window_draws_at_most_once_per_refresh_and_late_latches_a_burst() {
    for pacing in pacings() {
        if !expect(pacing).parks {
            continue;
        }
        let mut rig = Rig::open(pacing, Output::default());
        rig.run_until(rig.at(Duration::from_secs(1)));
        let from = rig.now;
        let interval = rig.interval();
        for step in 0..50u32 {
            rig.run_until(from + Duration::from_millis(u64::from(step)));
            rig.print();
        }
        rig.run_until(from + Duration::from_millis(60));
        let burst: Vec<Instant> = rig
            .presents
            .iter()
            .copied()
            .filter(|p| *p >= from)
            .collect();
        assert!(burst.len() >= 2, "{pacing:?}: the burst was drawn");
        for pair in burst.windows(2) {
            assert!(
                pair[1].duration_since(pair[0]) >= interval,
                "{pacing:?}: two frames {:?} apart inside one refresh",
                pair[1].duration_since(pair[0])
            );
        }
        assert!(
            burst.len() <= 50 * 1_000 / usize::try_from(interval.as_micros()).unwrap() + 2,
            "{pacing:?}: {} frames for a 50 ms burst",
            burst.len()
        );
        assert!(
            rig.scene.queued == 0 && !rig.scene.dirty,
            "{pacing:?}: the last ring of the burst reached the screen"
        );
    }
}

#[test]
fn two_idle_ticks_then_park() {
    let mut rig = Rig::open(FramePacing::Reactive(NonZeroU32::MAX), Output::default());
    rig.run_until(rig.at(Duration::from_secs(1)));
    rig.print();
    let drawn = rig.turns;
    assert!(matches!(rig.turnstile.pacer.heat(), Heat::Hot { .. }));
    rig.run_until(rig.at(Duration::from_secs(3)));
    assert_eq!(
        rig.turns - drawn,
        2,
        "after a frame the window takes exactly two idle ticks"
    );
    assert_eq!(rig.turnstile.pacer.heat(), Heat::Parked);
    assert_eq!(
        rig.turnstile.pacer.about_to_wait(rig.now).flow,
        Some(Flow::Wait)
    );
}

#[test]
fn pointer_motion_alone_never_keeps_a_window_hot() {
    for pacing in pacings() {
        if !expect(pacing).parks {
            continue;
        }
        let mut rig = Rig::open(pacing, Quiet);
        rig.run_until(rig.at(Duration::from_secs(1)));
        let presents = rig.presents.len();
        let from = rig.now;
        for step in 0..120u32 {
            rig.run_until(from + Duration::from_millis(u64::from(step) * 8));
            let before = rig.turns;
            rig.motion();
            assert!(
                rig.turns - before <= 1,
                "{pacing:?}: a pointer move costs at most the one turn that re-asks, not {}",
                rig.turns - before
            );
            assert_eq!(
                rig.turnstile.pacer.heat(),
                Heat::Parked,
                "{pacing:?}: a pointer move leaves the window parked"
            );
        }
        assert_eq!(
            rig.presents.len(),
            presents,
            "{pacing:?}: motion drew nothing"
        );
        assert_eq!(
            rig.turnstile.pacer.heat(),
            Heat::Parked,
            "{pacing:?}: and kept nothing Hot"
        );
        let after = rig.turns;
        rig.run_until(rig.now + Duration::from_secs(10));
        assert_eq!(
            rig.turns, after,
            "{pacing:?}: once the pointer stops, nothing ticks"
        );
    }
}

fn hidden_case(pacing: FramePacing, hide: fn(&mut Rig<Output>, bool)) {
    let mut rig = Rig::open(pacing, Output::default());
    rig.run_until(rig.at(Duration::from_secs(1)));
    hide(&mut rig, true);
    let acquires = rig.acquires();
    let drains = rig.drains;
    let from = rig.now;
    for step in 1..=20u32 {
        rig.run_until(from + Duration::from_millis(u64::from(step) * 50));
        rig.print();
    }
    rig.resize();
    rig.run_until(from + Duration::from_secs(5));
    let hidden_acquires = rig.acquires() - acquires;
    if expect(pacing).hides {
        assert_eq!(
            hidden_acquires, 0,
            "{pacing:?}: no acquire while Hidden, through 20 rings and a resize"
        );
        assert!(
            rig.drains - drains >= 20,
            "{pacing:?}: a hidden window still drains its output"
        );
        assert_eq!(
            rig.scene.queued, 0,
            "{pacing:?}: nothing printed while hidden is left undrained"
        );
        assert!(
            rig.turnstile.pacer.debts().any(),
            "{pacing:?}: the reveal is owed"
        );
        let revealed_at = rig.now;
        hide(&mut rig, false);
        assert_eq!(
            rig.presents.last().copied(),
            Some(revealed_at),
            "{pacing:?}: the reveal draws at once"
        );
        assert!(
            !rig.turnstile.pacer.debts().any(),
            "{pacing:?}: and settles every debt"
        );
    } else {
        assert!(
            hidden_acquires > 0,
            "{pacing:?} is a negative control: it acquires while hidden ({hidden_acquires})"
        );
    }
}

#[test]
fn an_occluded_window_acquires_nothing_and_owes_its_reveal() {
    for pacing in pacings() {
        hidden_case(pacing, Rig::occlude);
    }
}

#[test]
fn a_minimized_window_acquires_nothing_and_owes_its_reveal() {
    for pacing in pacings() {
        hidden_case(pacing, Rig::minimize);
    }
}

#[test]
fn a_reveal_draws_when_nothing_else_is_owed_and_the_renderer_answers_idle() {
    for hide in [Rig::occlude, Rig::minimize] {
        let mut rig = Rig::open(FramePacing::Reactive(NonZeroU32::MAX), Quiet);
        rig.run_until(rig.at(Duration::from_secs(1)));
        hide(&mut rig, true);
        assert!(rig.turnstile.pacer.hidden());
        rig.run_until(rig.at(Duration::from_secs(2)));
        let presents = rig.presents.len();
        let revealed_at = rig.now;
        hide(&mut rig, false);
        assert_eq!(
            rig.presents.len(),
            presents + 1,
            "the reveal is owed while hidden, so it draws although the renderer asks for nothing"
        );
        assert_eq!(rig.presents.last().copied(), Some(revealed_at));
    }
}

#[test]
fn a_window_occluded_before_its_first_frame_still_draws_it() {
    let start = Instant::now();
    let mut pacer = Pacer::new(FramePacing::Reactive(NonZeroU32::MAX), start);
    pacer.set_display_millihertz(Some(DISPLAY_MHZ));
    assert_eq!(pacer.set_occluded(true), Shift::Steady);
    assert!(
        pacer.decide(start, FrameDemand::Idle).is_some(),
        "the first frame is owed and a window nobody has seen yet is not Hidden"
    );
    pacer.presented(start);
    assert!(
        pacer.hidden(),
        "once shown, the recorded occlusion hides it"
    );
    assert!(pacer.debts().any(), "and the reveal is owed");
}

#[test]
fn on_wayland_a_withheld_redraw_reads_as_hidden_and_the_callback_reveals_it() {
    let mut rig = Rig::open_on(
        FramePacing::Reactive(NonZeroU32::MAX),
        Output::default(),
        true,
    );
    rig.run_until(rig.at(Duration::from_secs(1)));
    rig.withholding = true;
    rig.print();
    rig.run_until(rig.at(Duration::from_secs(2)));
    assert!(
        rig.turnstile.pacer.hidden(),
        "a redraw the compositor never released hid the window"
    );
    let acquires = rig.acquires();
    let drains = rig.drains;
    for step in 1..=10u32 {
        rig.run_until(
            rig.at(Duration::from_secs(2) + Duration::from_millis(u64::from(step) * 100)),
        );
        rig.print();
    }
    assert_eq!(
        rig.acquires(),
        acquires,
        "no acquire while the callbacks are withheld"
    );
    assert!(rig.drains - drains >= 10, "the output is still drained");
    assert_eq!(
        rig.scene.queued, 0,
        "every chunk printed while the callbacks are withheld was drained"
    );
    let wakes_before = rig.turns;
    rig.run_until(rig.at(Duration::from_secs(10)));
    assert_eq!(rig.turns, wakes_before, "a hidden window does not tick");
    let shown_at = rig.now;
    rig.withholding = false;
    rig.settle();
    assert!(
        !rig.turnstile.pacer.hidden(),
        "the callback that finally came revealed it"
    );
    assert_eq!(rig.presents.last().copied(), Some(shown_at));
}

#[test]
fn off_wayland_a_slow_redraw_never_hides_a_window() {
    let mut rig = Rig::open(FramePacing::Reactive(NonZeroU32::MAX), Output::default());
    rig.run_until(rig.at(Duration::from_secs(1)));
    rig.withholding = true;
    rig.print();
    rig.run_until(rig.at(Duration::from_secs(2)));
    assert!(!rig.turnstile.pacer.hidden());
}

#[test]
fn an_at_deadline_is_honoured_to_the_instant_with_no_wake_before_it() {
    for pacing in pacings() {
        let due = Instant::now() + Duration::from_millis(1_500);
        let mut rig = Rig::open(
            pacing,
            Wake2 {
                at: due,
                seen: false,
            },
        );
        let before = rig.turns;
        rig.run_until(due.checked_sub(Duration::from_millis(1)).unwrap());
        let early = rig.turns - before;
        rig.run_until(rig.at(Duration::from_secs(3)));
        let drawn: Vec<Instant> = rig
            .presents
            .iter()
            .copied()
            .filter(|p| *p > rig.start)
            .collect();
        if expect(pacing).parks {
            assert_eq!(drawn, vec![due], "{pacing:?}: one frame, at the deadline");
            assert!(
                early <= 1,
                "{pacing:?}: {early} turns before the deadline: an At parks at once"
            );
        } else {
            assert_eq!(drawn.len(), 1, "{pacing:?}: the deadline still draws once");
            assert!(
                drawn[0] >= due && drawn[0] < due + rig.interval(),
                "{pacing:?}: at its next tick"
            );
            assert!(early > 2, "{pacing:?} ticks while it waits ({early})");
        }
    }
}

#[test]
fn a_blinking_cursor_costs_its_flips_and_not_a_tick_rate() {
    for pacing in pacings() {
        let start = Instant::now();
        let scene = Blink {
            epoch: start,
            half: Duration::from_millis(500),
            drawn: None,
        };
        let mut rig = Rig::open(pacing, scene);
        let from = rig.turns;
        let presents = rig.presents.len();
        rig.run_until(rig.at(Duration::from_secs(10)));
        let flips = rig.presents.len() - presents;
        let turns = rig.turns - from;
        assert!(
            (19..=21).contains(&flips),
            "{pacing:?}: {flips} frames for 20 flips"
        );
        if expect(pacing).parks {
            for p in &rig.presents[presents..] {
                let into =
                    p.duration_since(start).as_nanos() % Duration::from_millis(500).as_nanos();
                assert_eq!(into, 0, "{pacing:?}: a flip is drawn at its instant");
            }
            assert!(
                turns <= 21 * 2,
                "{pacing:?}: {turns} turns for 20 flips: the flip's frame and the tick that parks on the next"
            );
        } else {
            assert!(turns > 500, "{pacing:?} ticks between flips ({turns})");
        }
    }
}

#[test]
fn a_kinetic_glide_draws_every_refresh_until_it_rests_then_parks() {
    let mut rig = Rig::open(
        FramePacing::Reactive(NonZeroU32::MAX),
        Glide { until: None },
    );
    rig.run_until(rig.at(Duration::from_secs(1)));
    let from = rig.now;
    rig.scene.until = Some(from + Duration::from_millis(300));
    rig.event(&AppEvent::Mouse(MouseEvent::Scroll {
        delta: crate::event::ScrollDelta::Lines { x: 0.0, y: -3.0 },
        modifiers: crate::event::Modifiers::default(),
    }));
    rig.run_until(from + Duration::from_secs(2));
    let interval = rig.interval();
    let glide: Vec<Instant> = rig
        .presents
        .iter()
        .copied()
        .filter(|p| *p >= from)
        .collect();
    let expected = 300_000 / interval.as_micros();
    assert!(
        (expected..=expected + 2).contains(&(glide.len() as u128)),
        "{} frames over a 300 ms glide at {interval:?}",
        glide.len()
    );
    assert_eq!(
        glide[0], from,
        "the glide's first frame is the scroll's own turn"
    );
    for pair in glide.windows(2) {
        assert_eq!(
            pair[1].duration_since(pair[0]),
            interval,
            "one frame a refresh"
        );
    }
    assert_eq!(
        rig.turnstile.pacer.heat(),
        Heat::Parked,
        "and at rest it parks"
    );
    assert_eq!(rig.presents_after(from + Duration::from_millis(320)), 0);
}

#[test]
fn a_resize_draws_even_when_the_renderer_answers_idle() {
    for pacing in pacings() {
        let mut rig = Rig::open(pacing, Quiet);
        rig.run_until(rig.at(Duration::from_secs(1)));
        let at = rig.now;
        rig.resize();
        assert_eq!(
            rig.presents.last().copied(),
            Some(at),
            "{pacing:?}: a resize draws"
        );
    }
}

#[test]
fn every_pacing_has_a_row() {
    for pacing in pacings() {
        let row = expect(pacing);
        assert_eq!(
            row.parks, row.hides,
            "{pacing:?}: parking and hiding come together"
        );
        assert_eq!(
            row.parks,
            Pacer::new(pacing, Instant::now()).reactive(),
            "{pacing:?}: only the demand scheduler parks"
        );
    }
}
