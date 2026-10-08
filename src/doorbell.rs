use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::task::{Wake, Waker};

use std::time::Instant;

use crate::pacer::{Pacer, Shift, Turn, Visibility};

pub(crate) trait Flag: Send + Sync + 'static {
    fn lowered() -> Self;
    fn raise(&self) -> bool;
    fn lower(&self);
}

impl Flag for AtomicBool {
    fn lowered() -> Self {
        AtomicBool::new(false)
    }

    fn raise(&self) -> bool {
        self.swap(true, Ordering::SeqCst)
    }

    fn lower(&self) {
        self.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct Latch<F> {
    pending: F,
}

impl<F: Flag> Latch<F> {
    pub(crate) fn new() -> Self {
        Self {
            pending: F::lowered(),
        }
    }

    pub(crate) fn ring(&self) -> bool {
        !self.pending.raise()
    }

    pub(crate) fn answer<R>(&self, drain: impl FnOnce() -> R) -> R {
        self.pending.lower();
        drain()
    }
}

pub(crate) type Line = Box<dyn Fn() + Send + Sync>;

struct Bell {
    latch: Latch<AtomicBool>,
    line: OnceLock<Line>,
}

impl Bell {
    fn ring(&self) {
        if self.latch.ring()
            && let Some(line) = self.line.get()
        {
            line();
        }
    }
}

impl Wake for Bell {
    fn wake(self: Arc<Self>) {
        self.ring();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.ring();
    }
}

pub(crate) struct Doorbell {
    bell: Arc<Bell>,
    visibility: Visibility,
}

impl Doorbell {
    pub(crate) fn new() -> Self {
        Self {
            bell: Arc::new(Bell {
                latch: Latch::new(),
                line: OnceLock::new(),
            }),
            visibility: Visibility::default(),
        }
    }

    pub(crate) fn waker(&self) -> Waker {
        Waker::from(Arc::clone(&self.bell))
    }

    pub(crate) fn visibility(&self) -> Visibility {
        self.visibility.clone()
    }

    pub(crate) fn connect(self, line: Line, mut pacer: Pacer) -> Turnstile {
        pacer.observed_by(self.visibility);
        let bell = self.bell;
        let _ = bell.line.set(line);
        bell.latch.answer(|| ());
        Turnstile { pacer, bell }
    }
}

pub(crate) trait Drains<Cx> {
    type Drained;
    fn turnstile(&mut self) -> &mut Turnstile;
    fn drain(&mut self, cx: Cx) -> Self::Drained;
}

pub(crate) struct Turnstile {
    pub(crate) pacer: Pacer,
    bell: Arc<Bell>,
}

impl Turnstile {
    pub(crate) fn redraw<Cx, H: Drains<Cx>>(host: &mut H, cx: Cx) -> H::Drained {
        let turnstile = host.turnstile();
        turnstile.pacer.redrawing();
        let bell = Arc::clone(&turnstile.bell);
        bell.latch.answer(|| host.drain(cx))
    }

    pub(crate) fn shift<Cx, H: Drains<Cx>>(
        host: &mut H,
        shift: Shift,
        cx: Cx,
    ) -> Option<H::Drained> {
        match shift {
            Shift::Hid => Some(Self::redraw(host, cx)),
            Shift::Steady | Shift::Shown => None,
        }
    }

    pub(crate) fn wait<Cx, H: Drains<Cx>>(
        host: &mut H,
        now: Instant,
        cx: Cx,
    ) -> (Turn, Option<H::Drained>) {
        let turn = host.turnstile().pacer.about_to_wait(now);
        let drained = turn.drain.then(|| Self::redraw(host, cx));
        (turn, drained)
    }

    #[cfg(test)]
    fn answer<R>(&self, drain: impl FnOnce() -> R) -> R {
        self.bell.latch.answer(drain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    fn idle() -> Pacer {
        Pacer::new(crate::FramePacing::Continuous, std::time::Instant::now())
    }

    fn counted() -> (Line, Arc<AtomicUsize>) {
        let sent = Arc::new(AtomicUsize::new(0));
        let line_sent = Arc::clone(&sent);
        (
            Box::new(move || {
                line_sent.fetch_add(1, Ordering::SeqCst);
            }),
            sent,
        )
    }

    #[test]
    fn rings_coalesce_until_the_loop_answers() {
        let bell = Doorbell::new();
        let waker = bell.waker();
        let (line, sent) = counted();
        let turnstile = bell.connect(line, idle());
        for _ in 0..50 {
            waker.wake_by_ref();
        }
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        turnstile.answer(|| ());
        waker.wake_by_ref();
        waker.wake_by_ref();
        assert_eq!(sent.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_waker_clone_shares_the_one_line() {
        let bell = Doorbell::new();
        let wakers: Vec<Waker> = (0..100).map(|_| bell.waker()).collect();
        let (line, sent) = counted();
        let turnstile = bell.connect(line, idle());
        for w in &wakers {
            let clone = w.clone();
            clone.wake_by_ref();
        }
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        turnstile.answer(|| ());
        wakers[37].wake_by_ref();
        assert_eq!(sent.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_ring_before_the_loop_exists_is_drained_by_its_first_answer_and_rings_again_after() {
        let bell = Doorbell::new();
        let waker = bell.waker();
        let queue = Arc::new(std::sync::Mutex::new(vec![1u8]));
        waker.wake_by_ref();
        let (line, sent) = counted();
        let turnstile = bell.connect(line, idle());
        assert_eq!(sent.load(Ordering::SeqCst), 0);
        let drained = turnstile.answer(|| std::mem::take(&mut *queue.lock().unwrap()));
        assert_eq!(drained, vec![1]);
        waker.wake_by_ref();
        assert_eq!(
            sent.load(Ordering::SeqCst),
            1,
            "connecting lowered the flag a pre-loop ring left raised"
        );
    }

    #[test]
    fn a_ring_from_another_thread_reaches_the_line() {
        let bell = Doorbell::new();
        let waker = bell.waker();
        let (tx, rx) = mpsc::channel::<()>();
        let tx = std::sync::Mutex::new(tx);
        let _turnstile = bell.connect(
            Box::new(move || {
                let _ = tx.lock().unwrap().send(());
            }),
            idle(),
        );
        std::thread::spawn(move || waker.wake()).join().unwrap();
        assert!(rx.recv_timeout(std::time::Duration::from_secs(1)).is_ok());
    }
}

#[cfg(test)]
mod loom_model {
    use super::{Flag, Latch};
    use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use loom::sync::{Arc, Mutex};

    impl Flag for AtomicBool {
        fn lowered() -> Self {
            AtomicBool::new(false)
        }

        fn raise(&self) -> bool {
            self.swap(true, Ordering::SeqCst)
        }

        fn lower(&self) {
            self.store(false, Ordering::SeqCst);
        }
    }

    struct Rig {
        latch: Latch<AtomicBool>,
        queue: Mutex<Vec<u8>>,
        rings: AtomicUsize,
    }

    impl Rig {
        fn produce(&self, item: u8) {
            self.queue.lock().unwrap().push(item);
            if self.latch.ring() {
                self.rings.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn take_ring(&self) -> bool {
            self.rings
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        }

        fn drain(&self) -> Vec<u8> {
            std::mem::take(&mut *self.queue.lock().unwrap())
        }
    }

    fn run(handle: fn(&Rig) -> Vec<u8>) {
        loom::model(move || {
            let rig = Arc::new(Rig {
                latch: Latch::new(),
                queue: Mutex::new(Vec::new()),
                rings: AtomicUsize::new(0),
            });
            let producer = {
                let rig = Arc::clone(&rig);
                loom::thread::spawn(move || {
                    rig.produce(1);
                    rig.produce(2);
                })
            };
            let mut seen = Vec::new();
            if rig.take_ring() {
                seen.extend(handle(&rig));
            }
            producer.join().unwrap();
            while rig.take_ring() {
                seen.extend(handle(&rig));
            }
            seen.sort_unstable();
            assert_eq!(
                seen,
                vec![1, 2],
                "an item was left behind a ring nobody answered"
            );
        });
    }

    #[test]
    fn no_wake_is_lost_when_a_ring_races_the_drain() {
        run(|rig| rig.latch.answer(|| rig.drain()));
    }

    #[test]
    #[should_panic(expected = "an item was left behind a ring nobody answered")]
    fn lowering_the_flag_after_the_drain_loses_a_wake() {
        run(|rig| {
            let drained = rig.drain();
            rig.latch.answer(|| ());
            drained
        });
    }
}
