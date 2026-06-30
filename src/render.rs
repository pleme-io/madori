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

/// Trait that applications implement for custom rendering.
pub trait RenderCallback: Send + 'static {
    /// Called each frame. Draw into `ctx.surface_view`.
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
