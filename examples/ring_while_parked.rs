use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Parked {
    dirty: Arc<AtomicBool>,
}

impl madori::RenderCallback for Parked {
    fn needs_frame(&mut self, _q: madori::FrameQuery) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }

    fn render(&mut self, ctx: &mut madori::RenderContext<'_>) {
        let mut encoder = ctx
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ctx.surface_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        }));
        ctx.gpu.queue.submit(Some(encoder.finish()));
    }
}

fn quantile(sorted: &[f64], percent: usize) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[(sorted.len() - 1) * percent / 100]
}

fn report(wake_off: bool, rings: usize, seen: &Mutex<Vec<f64>>) {
    let mut seen = seen.lock().unwrap().clone();
    seen.sort_by(f64::total_cmp);
    println!(
        "wake_off={wake_off} rings={rings} redraws_with_a_ring={} ring_to_redraw_us p50={:.1} p90={:.1} max={:.1}",
        seen.len(),
        quantile(&seen, 50),
        quantile(&seen, 90),
        seen.last().copied().unwrap_or(f64::NAN)
    );
}

fn main() -> Result<(), madori::MadoriError> {
    let wake_off = std::env::args().any(|a| a == "--wake-off");
    let rings = 20;
    let rung_at: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let seen: Arc<Mutex<Vec<f64>>> = Arc::default();
    let dirty = Arc::new(AtomicBool::new(false));
    let window = madori::App::builder(Parked {
        dirty: Arc::clone(&dirty),
    });
    let waker = if wake_off {
        std::task::Waker::noop().clone()
    } else {
        window.waker()
    };
    {
        let rung_at = Arc::clone(&rung_at);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1_500));
            for _ in 0..rings {
                std::thread::sleep(Duration::from_millis(150));
                rung_at.lock().unwrap().push(Instant::now());
                waker.wake_by_ref();
            }
        });
    }
    {
        let seen = Arc::clone(&seen);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(12));
            report(wake_off, rings, &seen);
            std::process::exit(0);
        });
    }
    let started = Instant::now();
    let seen_by_loop = Arc::clone(&seen);
    let dirty_by_loop = Arc::clone(&dirty);
    window
        .title("ring_while_parked")
        .size(200, 120)
        .frame_pacing(madori::FramePacing::Reactive(
            NonZeroU32::new(60).expect("non-zero"),
        ))
        .on_event(move |event, _renderer| {
            let mut response = madori::EventResponse::default();
            if matches!(event, madori::AppEvent::RedrawRequested) {
                let now = Instant::now();
                let drained = std::mem::take(&mut *rung_at.lock().unwrap());
                if !drained.is_empty() {
                    dirty_by_loop.store(true, Ordering::SeqCst);
                }
                let mut seen = seen_by_loop.lock().unwrap();
                seen.extend(
                    drained
                        .into_iter()
                        .map(|at| now.duration_since(at).as_secs_f64() * 1e6),
                );
                response.exit = seen.len() >= rings || started.elapsed() > Duration::from_secs(10);
            }
            response
        })
        .run()?;
    report(wake_off, rings, &seen);
    Ok(())
}
