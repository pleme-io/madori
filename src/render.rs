use garasu::{GpuContext, TextLayerStack};
use ishou_tokens::{ColorPalette, FleetTheme, Srgb};

/// Context passed to the application's render callback each frame.
pub struct RenderContext<'a> {
    /// The GPU context (device, queue, etc).
    pub gpu: &'a GpuContext,
    /// The text layer stack for drawing text. Each independent text surface
    /// (terminal grid, overlays) prepares/renders its OWN layer so a second
    /// pass cannot clobber the first's vertex buffer (see
    /// `garasu::TextLayerStack`). Single-pass consumers may use the back-compat
    /// `text.prepare(..)` / `text.render(..)` methods unchanged.
    pub text: &'a mut TextLayerStack,
    /// Current surface texture view to render into.
    pub surface_view: &'a wgpu::TextureView,
    /// Current window dimensions in physical pixels.
    pub width: u32,
    pub height: u32,
    /// HiDPI scale factor — logical pixels × `scale_factor` = physical
    /// pixels. Renderers that author dimensions / positions in logical
    /// units (`font_size`, `padding`, etc.) must multiply by this
    /// before drawing into the surface, which is sized in physical
    /// pixels. Updated on `ScaleFactorChanged` events.
    pub scale_factor: f64,
    /// Time since app start in seconds.
    pub elapsed: f32,
    /// Delta time since last frame in seconds.
    pub dt: f32,
}

/// What a renderer is told when asked whether it needs a frame.
///
/// The same clock [`RenderContext`] carries, handed over BEFORE the swapchain
/// acquire. It is here because the honest answer to "do you need a frame?" is
/// often time-dependent — a blinking cursor, a decaying flash, any animation
/// in flight — and a renderer with no clock would have to answer `true` on
/// every tick just in case, which is the behaviour this exists to end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameQuery {
    /// Seconds since app start. Same value [`RenderContext::elapsed`] will
    /// carry if a frame follows.
    pub elapsed: f32,
    /// Seconds since the last frame that was actually PRESENTED — not since
    /// the last tick. Skipped frames widen it, which is what an animation
    /// stepping by `dt` needs: the motion is the same whether it was sampled
    /// often or rarely.
    pub dt: f32,
}

/// Trait that applications implement for custom rendering.
pub trait RenderCallback: Send + 'static {
    /// Is a frame needed at all?
    ///
    /// Asked **before** the swapchain image is acquired, so returning `false`
    /// skips acquire, [`render`](Self::render) **and** present together. The
    /// window keeps showing the last presented frame, which is what it should
    /// show when nothing has changed.
    ///
    /// ★ **The three must move together, and that is the whole point.** A
    /// renderer cannot implement this itself: `madori` owns the acquire and
    /// the present, so a renderer that decided to skip could only skip its
    /// own drawing — and skipping the draw while `madori` still presents
    /// hands the compositor a swapchain slot nobody painted. That surfaces
    /// content from 2–3 frames back (the "prompt leaves shadows of itself"
    /// regression) and, on Metal, an uninitialised slot's magenta.
    ///
    /// `mado` hit exactly that, and drew the wrong conclusion from it: it
    /// went back to painting every frame unconditionally, keeping the skip
    /// decision as a counter and a log line while rendering anyway
    /// (`TOTAL_FRAMES_SKIPPED` read 9,934,969 of 10,726,562, none of which
    /// were skipped). Measured on plo 2026-08-21: **50.7% of a core on a
    /// static screen**, against a source comment estimating "≈0.2% … free
    /// correctness with no measurable cost".
    ///
    /// The implication of *never present an unwritten slot* is **do not
    /// present**, not **always write** — and only this trait, on this side of
    /// the seam, can express that.
    ///
    /// Defaults to `true`, so every existing renderer behaves exactly as
    /// before. Takes `&mut self` so an implementation may drain a dirty flag.
    fn needs_frame(&mut self, _q: FrameQuery) -> bool {
        true
    }

    /// Called each frame. Draw into `ctx.surface_view`.
    ///
    /// Not called at all when [`needs_frame`](Self::needs_frame) returns
    /// `false`.
    fn render(&mut self, ctx: &mut RenderContext<'_>);

    /// Called when the window is resized.
    fn resize(&mut self, _width: u32, _height: u32) {}

    /// Called once after GPU is initialized, before first render.
    fn init(&mut self, _gpu: &GpuContext) {}
}

/// A no-op renderer that clears to a background color.
pub struct ClearRenderer {
    pub color: wgpu::Color,
}

impl Default for ClearRenderer {
    fn default() -> Self {
        // Nord polar-night background, sourced from the ishou fleet design
        // system instead of a hand-authored literal. `FleetTheme::PlemeDark`
        // is the Nord Polar Night theme; its resolved `background` is the
        // canonical `#2E3440` (nord0) the original literal approximated.
        //
        // The colour is routed through the typed `Srgb → Linear →
        // wgpu::Color` path. madori configures an sRGB-storage surface
        // (`app.rs` selects the format via `is_srgb()`), so a clear colour
        // must be supplied in LINEAR space — passing the sRGB unit-floats
        // verbatim (as the old literal did) makes the GPU gamma-encode them
        // on store and renders washed-out grey. `Srgb::to_linear()` is the
        // one type-correct construction path (ishou-tokens `space.rs`).
        //
        // The unwrap is unreachable — `ResolvedTheme::pleme_dark().background`
        // is always valid hex — but the fallback stays fully token-sourced
        // (`ColorPalette::pleme().polar_night_0`, the same nord0 value) so no
        // hand-authored hex survives at the paint site.
        let background = FleetTheme::PlemeDark.resolve().background;
        let color = Srgb::from_hex(&background)
            .unwrap_or_else(|| ColorPalette::pleme().polar_night_0.into())
            .to_linear()
            .with_alpha(1.0)
            .into();
        Self { color }
    }
}

impl RenderCallback for ClearRenderer {
    fn render(&mut self, ctx: &mut RenderContext<'_>) {
        let mut encoder = ctx
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clear"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: ctx.surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        ctx.gpu.queue.submit(std::iter::once(encoder.finish()));
    }
}

#[cfg(test)]
mod needs_frame_tests {
    use super::{FrameQuery, RenderCallback, RenderContext};

    const Q: FrameQuery = FrameQuery { elapsed: 0.0, dt: 0.0 };

    /// A renderer that only implements `render` — i.e. every consumer that
    /// existed before `needs_frame` was added.
    struct Legacy;
    impl RenderCallback for Legacy {
        fn render(&mut self, _ctx: &mut RenderContext<'_>) {}
    }

    /// A renderer that reports itself clean forever.
    struct AlwaysClean;
    impl RenderCallback for AlwaysClean {
        fn needs_frame(&mut self, _q: FrameQuery) -> bool {
            false
        }
        fn render(&mut self, _ctx: &mut RenderContext<'_>) {}
    }

    /// A renderer whose dirty flag is CONSUMED by the question, which is the
    /// shape a real one has.
    struct Draining(bool);
    impl RenderCallback for Draining {
        fn needs_frame(&mut self, _q: FrameQuery) -> bool {
            std::mem::replace(&mut self.0, false)
        }
        fn render(&mut self, _ctx: &mut RenderContext<'_>) {}
    }

    #[test]
    fn the_default_is_draw_because_the_alternative_freezes_every_consumer() {
        // ★ THIS IS THE LOAD-BEARING ONE. `needs_frame` was added with a
        // default so no existing renderer had to change. If that default ever
        // became `false`, every consumer that has not overridden it would
        // stop drawing entirely — a black window with no error, no panic and
        // no log line, in a crate whose consumers are other repositories.
        assert!(
            Legacy.needs_frame(Q),
            "the default must be `true`: a renderer that never opted in must \
             keep drawing exactly as it did before this trait method existed"
        );
    }

    #[test]
    fn an_override_is_honoured_in_both_directions() {
        assert!(!AlwaysClean.needs_frame(Q));
        let mut d = Draining(true);
        assert!(d.needs_frame(Q), "the first ask sees the dirty flag");
        assert!(!d.needs_frame(Q), "and the ask CONSUMED it");
    }

    #[test]
    fn the_question_may_be_asked_through_a_generic_bound() {
        // `App` holds `R: RenderCallback` and asks through that bound, so a
        // default method that somehow failed to dispatch generically would
        // break there and nowhere else.
        fn ask<R: RenderCallback>(r: &mut R) -> bool {
            r.needs_frame(Q)
        }
        assert!(ask(&mut Legacy));
        assert!(!ask(&mut AlwaysClean));
    }
}
