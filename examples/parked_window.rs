use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Counts {
    turns: AtomicU64,
    asks: AtomicU64,
    frames: AtomicU64,
}

struct Parked {
    dirty: Arc<AtomicBool>,
    counts: Arc<Counts>,
}

impl madori::RenderCallback for Parked {
    fn frame_demand(&mut self, _q: madori::FrameQuery) -> madori::FrameDemand {
        self.counts.asks.fetch_add(1, Ordering::Relaxed);
        if self.dirty.swap(false, Ordering::SeqCst) {
            madori::FrameDemand::Now
        } else {
            madori::FrameDemand::Idle
        }
    }

    fn render(&mut self, ctx: &mut madori::RenderContext<'_>) {
        self.counts.frames.fetch_add(1, Ordering::Relaxed);
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

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn pacing(name: &str) -> madori::FramePacing {
    match name {
        "capped" => madori::FramePacing::Capped(NonZeroU32::new(60).expect("non-zero")),
        "continuous" => madori::FramePacing::Continuous,
        _ => madori::FramePacing::Reactive(NonZeroU32::MAX),
    }
}

fn report(counts: &Counts, visibility: &madori::Visibility, started: Instant, pacing: &str) {
    println!(
        "{{\"pacing\":\"{pacing}\",\"secs\":{:.3},\"turns\":{},\"asks\":{},\"frames\":{},\"hidden\":{},\"hides\":{},\"reveals\":{}}}",
        started.elapsed().as_secs_f64(),
        counts.turns.load(Ordering::Relaxed),
        counts.asks.load(Ordering::Relaxed),
        counts.frames.load(Ordering::Relaxed),
        visibility.hidden(),
        visibility.hides(),
        visibility.reveals()
    );
}

fn main() -> Result<(), madori::MadoriError> {
    let pacing_name = arg("--pacing").unwrap_or_else(|| "reactive".into());
    let secs: u64 = arg("--secs").map_or(70, |s| s.parse().expect("--secs N"));
    let print_every = arg("--print-every-ms")
        .map(|s| Duration::from_millis(s.parse().expect("--print-every-ms N")));
    let dirty = Arc::new(AtomicBool::new(false));
    let counts = Arc::new(Counts::default());
    let window = madori::App::builder(Parked {
        dirty: Arc::clone(&dirty),
        counts: Arc::clone(&counts),
    });
    let waker = window.waker();
    let visibility = window.visibility();
    let started = Instant::now();
    if let Some(every) = print_every {
        let dirty = Arc::clone(&dirty);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(every);
                dirty.store(true, Ordering::SeqCst);
                waker.wake_by_ref();
            }
        });
    }
    {
        let counts = Arc::clone(&counts);
        let visibility = visibility.clone();
        let pacing_name = pacing_name.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(secs));
            report(&counts, &visibility, started, &pacing_name);
            std::process::exit(0);
        });
    }
    let counts_by_loop = Arc::clone(&counts);
    window
        .title("parked_window")
        .size(320, 200)
        .frame_pacing(pacing(&pacing_name))
        .on_event(move |event, _renderer| {
            if matches!(event, madori::AppEvent::RedrawRequested) {
                counts_by_loop.turns.fetch_add(1, Ordering::Relaxed);
            }
            madori::EventResponse::default()
        })
        .run()?;
    report(&counts, &visibility, started, &pacing_name);
    Ok(())
}
