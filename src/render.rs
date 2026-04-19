use garasu::{GpuContext, TextRenderer};

/// Context passed to the application's render callback each frame.
pub struct RenderContext<'a> {
    /// The GPU context (device, queue, etc).
    pub gpu: &'a GpuContext,
    /// The text renderer for drawing text.
    pub text: &'a mut TextRenderer,
    /// Current surface texture view to render into.
    pub surface_view: &'a wgpu::TextureView,
    /// Current window dimensions in physical pixels.
    pub width: u32,
    pub height: u32,
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
        // Nord polar night background
        Self {
            color: wgpu::Color {
                r: 0.180,
                g: 0.204,
                b: 0.251,
                a: 1.0,
            },
        }
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
