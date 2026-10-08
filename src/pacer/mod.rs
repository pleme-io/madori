use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::app::{FrameDebt, FrameDebts, FramePacing, frame_gate};
use crate::render::FrameDemand;

#[cfg(test)]
mod matrix;
#[cfg(test)]
mod tests;

pub(crate) const IDLE_TICKS: u8 = 2;
const WITHHELD_FRAMES: u32 = 4;
const WITHHELD_FLOOR: Duration = Duration::from_millis(50);
const FALLBACK_HZ: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Poll,
    Wait,
    WaitUntil(Instant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Turn {
    pub(crate) redraw: bool,
    pub(crate) drain: bool,
    pub(crate) flow: Option<Flow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wake {
    Redraw,
    Defer,
    Drain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shift {
    Steady,
    Hid,
    Shown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Heat {
    Parked,
    Hot { idle: u8 },
}

#[derive(Debug)]
pub struct Visible {
    sealed: (),
}

#[derive(Debug, Default)]
struct Shown {
    hidden: AtomicBool,
    hides: AtomicU64,
    reveals: AtomicU64,
}

#[derive(Debug, Clone, Default)]
pub struct Visibility(Arc<Shown>);

impl Visibility {
    #[must_use]
    pub fn hidden(&self) -> bool {
        self.0.hidden.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn hides(&self) -> u64 {
        self.0.hides.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn reveals(&self) -> u64 {
        self.0.reveals.load(Ordering::Relaxed)
    }

    fn hid(&self) {
        self.0.hidden.store(true, Ordering::Relaxed);
        self.0.hides.fetch_add(1, Ordering::Relaxed);
    }

    fn revealed(&self) {
        self.0.hidden.store(false, Ordering::Relaxed);
        self.0.reveals.fetch_add(1, Ordering::Relaxed);
    }
}

pub trait Surface {
    type Frame;
    type Error;

    #[allow(clippy::missing_errors_doc)]
    fn acquire(&self, visible: Visible) -> Result<Self::Frame, Self::Error>;
}

impl Surface for wgpu::Surface<'_> {
    type Frame = wgpu::SurfaceTexture;
    type Error = wgpu::SurfaceError;

    fn acquire(&self, visible: Visible) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError> {
        let Visible { sealed: () } = visible;
        self.get_current_texture()
    }
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Pacer {
    // Minimum gap between frames, or None for the legacy
    // spin-as-fast-as-possible loop. `None` is the default and
    // keeps `ControlFlow::Poll` + the self-sustaining
    // request_redraw() at the end of RedrawRequested; `Some(d)`
    // swaps both for a WaitUntil deadline driven from
    // about_to_wait. See `FramePacing`.
    interval: Option<Duration>,
    ceiling: Option<Duration>,
    reactive: bool,
    heat: Heat,
    // When the next frame is due. Meaningless (and never read)
    // while `interval` is None.
    next_frame_due: Instant,
    ring_due: Option<Instant>,
    deadline: Option<Instant>,
    last_present: Instant,
    debts: FrameDebts,
    shown: bool,
    occluded: bool,
    minimized: bool,
    infers_withheld: bool,
    withheld: bool,
    requested_at: Option<Instant>,
    visibility: Visibility,
}

impl Pacer {
    pub(crate) fn new(pacing: FramePacing, now: Instant) -> Self {
        let interval = pacing.frame_interval();
        let mut pacer = Self {
            interval,
            ceiling: interval,
            reactive: matches!(pacing, FramePacing::Reactive(_)),
            heat: Heat::Hot { idle: 0 },
            next_frame_due: now,
            ring_due: None,
            deadline: None,
            last_present: now,
            debts: FrameDebts::at_startup(),
            shown: false,
            occluded: false,
            minimized: false,
            infers_withheld: false,
            withheld: false,
            requested_at: None,
            visibility: Visibility::default(),
        };
        pacer.set_display_millihertz(None);
        pacer
    }

    pub(crate) fn observed_by(&mut self, visibility: Visibility) {
        self.visibility = visibility;
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

    pub(crate) fn reactive(&self) -> bool {
        self.reactive
    }

    #[cfg(test)]
    pub(crate) fn heat(&self) -> Heat {
        self.heat
    }

    #[cfg(test)]
    pub(crate) fn interval(&self) -> Option<Duration> {
        self.interval
    }

    #[cfg(test)]
    pub(crate) fn debts(&self) -> FrameDebts {
        self.debts
    }

    pub(crate) fn owe(&mut self, debt: FrameDebt) {
        self.debts.owe(debt);
    }

    pub(crate) fn set_display_millihertz(&mut self, millihertz: Option<u32>) {
        if !self.reactive {
            return;
        }
        let display = millihertz
            .filter(|mhz| *mhz > 0)
            .map_or(Duration::from_secs_f64(1.0 / FALLBACK_HZ), |mhz| {
                Duration::from_secs_f64(1_000.0 / f64::from(mhz))
            });
        self.interval = self.ceiling.map(|ceiling| ceiling.max(display));
    }

    pub(crate) fn infer_withheld_redraws(&mut self, infers: bool) {
        self.infers_withheld = infers && self.reactive;
    }

    pub(crate) fn hidden(&self) -> bool {
        self.reactive && self.shown && (self.occluded || self.minimized || self.withheld)
    }

    fn shifted(&mut self, was_hidden: bool) -> Shift {
        match (was_hidden, self.hidden()) {
            (false, true) => {
                self.heat = Heat::Parked;
                self.ring_due = None;
                self.deadline = None;
                self.debts.owe(FrameDebt::Revealed);
                self.visibility.hid();
                tracing::info!(
                    target: "madori::pacer",
                    occluded = self.occluded,
                    minimized = self.minimized,
                    withheld = self.withheld,
                    "window hidden: no frame is acquired until it is shown"
                );
                Shift::Hid
            }
            (true, false) => {
                self.visibility.revealed();
                tracing::info!(target: "madori::pacer", "window shown: the reveal draws at once");
                Shift::Shown
            }
            _ => Shift::Steady,
        }
    }

    pub(crate) fn set_occluded(&mut self, occluded: bool) -> Shift {
        let was = self.hidden();
        self.occluded = occluded;
        self.shifted(was)
    }

    pub(crate) fn set_minimized(&mut self, minimized: bool) -> Shift {
        let was = self.hidden();
        self.minimized = minimized;
        self.shifted(was)
    }

    pub(crate) fn requested(&mut self, now: Instant) {
        if self.infers_withheld {
            self.requested_at.get_or_insert(now);
        }
    }

    pub(crate) fn arrived(&mut self) {
        self.requested_at = None;
        let was = self.hidden();
        self.withheld = false;
        let _ = self.shifted(was);
    }

    pub(crate) fn ring(&mut self, now: Instant) -> Wake {
        if self.hidden() {
            return Wake::Drain;
        }
        let Some(interval) = self.interval else {
            return Wake::Redraw;
        };
        let slot = self.last_present + interval;
        if now >= slot {
            self.ring_due = None;
            return Wake::Redraw;
        }
        self.ring_due = Some(self.ring_due.map_or(slot, |due| due.min(slot)));
        Wake::Defer
    }

    pub(crate) fn reask(&mut self, now: Instant) -> bool {
        if !self.reactive {
            return true;
        }
        matches!(self.ring(now), Wake::Redraw)
    }

    pub(crate) fn redrawing(&mut self) {
        self.ring_due = None;
    }

    pub(crate) fn decide(&mut self, now: Instant, demand: FrameDemand) -> Option<Visible> {
        if self.hidden() {
            return None;
        }
        let draw = frame_gate(self.debts, demand.draws());
        if self.reactive {
            self.deadline = demand.deadline().filter(|at| *at > now);
            if !draw && let Heat::Hot { idle } = self.heat {
                let idle = idle.saturating_add(1);
                self.heat = if idle >= IDLE_TICKS || self.deadline.is_some() {
                    Heat::Parked
                } else {
                    Heat::Hot { idle }
                };
            }
        }
        draw.then_some(Visible { sealed: () })
    }

    pub(crate) fn presented(&mut self, now: Instant) {
        self.debts.settle();
        self.last_present = now;
        if self.reactive {
            self.heat = Heat::Hot { idle: 0 };
            if let Some(interval) = self.interval {
                self.next_frame_due = now + interval;
            }
        }
        let was = self.hidden();
        self.shown = true;
        let _ = self.shifted(was);
    }

    pub(crate) fn about_to_wait(&mut self, now: Instant) -> Turn {
        let Some(interval) = self.interval else {
            return Turn {
                redraw: false,
                drain: false,
                flow: None,
            };
        };
        if self.reactive {
            return self.demand_turn(now, interval);
        }
        let rung = self.ring_due.is_some_and(|due| now >= due);
        if rung {
            self.ring_due = None;
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
            drain: false,
            flow: Some(Flow::WaitUntil(wake)),
        }
    }

    fn demand_turn(&mut self, now: Instant, interval: Duration) -> Turn {
        let parked = Turn {
            redraw: false,
            drain: false,
            flow: Some(Flow::Wait),
        };
        if self.hidden() {
            self.next_frame_due = now;
            return parked;
        }
        let withheld_by = self
            .requested_at
            .filter(|_| self.shown)
            .map(|at| at + (interval * WITHHELD_FRAMES).max(WITHHELD_FLOOR));
        if withheld_by.is_some_and(|by| now >= by) {
            let was = self.hidden();
            self.withheld = true;
            self.next_frame_due = now;
            return Turn {
                drain: self.shifted(was) == Shift::Hid,
                ..parked
            };
        }
        let mut redraw = false;
        if self.ring_due.is_some_and(|due| now >= due) {
            self.ring_due = None;
            redraw = true;
        }
        if self.deadline.is_some_and(|at| now >= at) {
            self.deadline = None;
            redraw = true;
        }
        let tick = match self.heat {
            Heat::Parked => {
                self.next_frame_due = now;
                None
            }
            Heat::Hot { .. } => {
                if now >= self.next_frame_due {
                    let mut next = self.next_frame_due + interval;
                    if next <= now {
                        next = now + interval;
                    }
                    self.next_frame_due = next;
                    redraw = true;
                }
                Some(self.next_frame_due)
            }
        };
        let wake = [self.ring_due, self.deadline, tick, withheld_by]
            .into_iter()
            .flatten()
            .min();
        Turn {
            redraw,
            drain: false,
            flow: Some(wake.map_or(Flow::Wait, Flow::WaitUntil)),
        }
    }
}
