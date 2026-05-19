use crate::error::{MadoriError, Result};
use crate::event::{
    AppEvent, EventResponse, ImeEvent, KeyCode, KeyEvent, Modifiers, MouseButton, MouseEvent,
};
use crate::render::{RenderCallback, RenderContext};
use garasu::GpuContext;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Configuration for creating an App.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
    pub vsync: bool,
    pub transparent: bool,
    /// Show server/window-manager decorations (titlebar, border,
    /// close/minimize buttons). When `false`, winit creates a
    /// borderless window — the entire surface is the
    /// application's canvas.
    ///
    /// On macOS, leaving this `true` is usually preferable: it
    /// keeps the traffic-light buttons usable while platform-
    /// specific code (e.g. mado's `platform::apply_native_styling`)
    /// can flip `FullSizeContentView` + transparent titlebar to
    /// integrate the chrome into the content area. Setting
    /// `false` on macOS also removes the traffic lights — a
    /// genuinely chromeless look that many operators prefer for
    /// kiosk / minimal modes.
    ///
    /// Default: `true` (preserves legacy behavior). Mado's
    /// platform-aware default picks `true` on macOS and `false`
    /// elsewhere via its own `default_decorations()` helper.
    pub decorations: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: String::from("pleme-io"),
            width: 1280,
            height: 720,
            resizable: true,
            vsync: true,
            transparent: false,
            decorations: true,
        }
    }
}

/// Builder for constructing an App with fluent API.
pub struct AppBuilder<R: RenderCallback> {
    pub config: AppConfig,
    renderer: R,
    event_handler: Option<Box<dyn FnMut(&AppEvent, &mut R) -> EventResponse + Send + 'static>>,
}

impl<R: RenderCallback> AppBuilder<R> {
    pub fn new(renderer: R) -> Self {
        Self {
            config: AppConfig::default(),
            renderer,
            event_handler: None,
        }
    }

    #[must_use]
    pub fn config(mut self, config: AppConfig) -> Self {
        self.config = config;
        self
    }

    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.config.title = title.into();
        self
    }

    #[must_use]
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.config.width = width;
        self.config.height = height;
        self
    }

    /// Set event handler. Return `EventResponse` to control behavior.
    /// For backwards compatibility, closures returning `bool` are also accepted
    /// via the `From<bool>` impl on `EventResponse`.
    #[must_use]
    pub fn on_event<F, Resp>(mut self, mut handler: F) -> Self
    where
        F: FnMut(&AppEvent, &mut R) -> Resp + Send + 'static,
        Resp: Into<EventResponse>,
    {
        self.event_handler = Some(Box::new(move |event, renderer| {
            handler(event, renderer).into()
        }));
        self
    }

    /// Build and run the application. This blocks until the window is closed.
    pub fn run(self) -> Result<()> {
        App::run_inner(self.config, self.renderer, self.event_handler)
    }
}

/// The main application entry point.
pub struct App;

impl App {
    /// Create a builder with custom renderer.
    pub fn builder<R: RenderCallback>(renderer: R) -> AppBuilder<R> {
        AppBuilder::new(renderer)
    }

    fn run_inner<R: RenderCallback>(
        config: AppConfig,
        renderer: R,
        event_handler: Option<Box<dyn FnMut(&AppEvent, &mut R) -> EventResponse + Send + 'static>>,
    ) -> Result<()> {
        use winit::application::ApplicationHandler;
        use winit::event::{ElementState, WindowEvent};
        use winit::event_loop::EventLoop;
        use winit::window::{Window, WindowAttributes};

        struct Handler<R: RenderCallback> {
            config: AppConfig,
            renderer: R,
            event_handler:
                Option<Box<dyn FnMut(&AppEvent, &mut R) -> EventResponse + Send + 'static>>,
            window: Option<std::sync::Arc<Window>>,
            gpu: Option<GpuContext>,
            text: Option<garasu::TextRenderer>,
            surface: Option<wgpu::Surface<'static>>,
            surface_config: Option<wgpu::SurfaceConfiguration>,
            start_time: Instant,
            last_frame: Instant,
            modifiers: winit::keyboard::ModifiersState,
            width: u32,
            height: u32,
            // HiDPI scale factor of the active window — captured at
            // resume + refreshed on ScaleFactorChanged. Renderers that
            // author dimensions in logical pixels must multiply by
            // this before drawing into the physical-pixel surface.
            scale_factor: f64,
            // Track cursor position for mouse button events
            cursor_x: f64,
            cursor_y: f64,
            // Window starts hidden so the user doesn't see the
            // uninitialised swapchain (random GPU memory shows up as a
            // multicolor purple flash on macOS Metal). Flipped to true
            // after the first frame is presented.
            first_frame_presented: bool,
        }

        impl<R: RenderCallback> Handler<R> {
            fn dispatch(
                &mut self,
                event: &AppEvent,
                event_loop: &winit::event_loop::ActiveEventLoop,
            ) -> EventResponse {
                let resp = self
                    .event_handler
                    .as_mut()
                    .map_or(EventResponse::default(), |h| (h)(event, &mut self.renderer));

                // Handle set_title
                if let Some(title) = &resp.set_title {
                    if let Some(w) = &self.window {
                        w.set_title(title);
                    }
                }

                // Handle cursor visibility
                if let Some(visible) = resp.set_cursor_visible {
                    if let Some(w) = &self.window {
                        w.set_cursor_visible(visible);
                    }
                }

                // Handle fullscreen toggle
                if resp.toggle_fullscreen {
                    if let Some(w) = &self.window {
                        use winit::window::Fullscreen;
                        if w.fullscreen().is_some() {
                            w.set_fullscreen(None);
                        } else {
                            w.set_fullscreen(Some(Fullscreen::Borderless(None)));
                        }
                    }
                }

                // Handle exit request
                if resp.exit {
                    event_loop.exit();
                }

                resp
            }
        }

        impl<R: RenderCallback> ApplicationHandler for Handler<R> {
            fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
                if self.window.is_some() {
                    return;
                }

                let attrs = WindowAttributes::default()
                    .with_title(&self.config.title)
                    // Logical pixels are scale-aware: 1200x800 logical renders at
                    // 2400x1600 physical on a 2x HiDPI display (the user-perceived
                    // "normal big window"), not the cramped 600x400-logical area
                    // PhysicalSize would have produced. winit handles the
                    // scale-factor multiplication internally.
                    .with_inner_size(winit::dpi::LogicalSize::new(
                        self.config.width,
                        self.config.height,
                    ))
                    .with_resizable(self.config.resizable)
                    .with_transparent(self.config.transparent)
                    .with_decorations(self.config.decorations)
                    // Start hidden — the swapchain backbuffer holds
                    // uninitialised GPU memory until the first present.
                    // Showing the window before that lets the user see
                    // random bytes as a multicolor purple flash on
                    // macOS Metal. set_visible(true) after first frame.
                    .with_visible(false);

                let window = match event_loop.create_window(attrs) {
                    Ok(w) => std::sync::Arc::new(w),
                    Err(e) => {
                        tracing::error!("failed to create window: {e}");
                        event_loop.exit();
                        return;
                    }
                };

                // Enable IME
                window.set_ime_allowed(true);

                let size = window.inner_size();
                self.width = size.width;
                self.height = size.height;
                self.scale_factor = window.scale_factor();

                // Initialize GPU
                match pollster::block_on(GpuContext::new()) {
                    Ok(gpu) => {
                        let surface = gpu
                            .instance
                            .create_surface(window.clone())
                            .expect("failed to create surface");

                        let caps = surface.get_capabilities(&gpu.adapter);
                        let format = caps
                            .formats
                            .iter()
                            .find(|f| f.is_srgb())
                            .copied()
                            .unwrap_or(caps.formats[0]);

                        let present_mode = if self.config.vsync {
                            wgpu::PresentMode::AutoVsync
                        } else {
                            wgpu::PresentMode::AutoNoVsync
                        };

                        // Prefer Opaque alpha_mode when the surface
                        // advertises it. On macOS the default
                        // (caps.alpha_modes[0]) is sometimes
                        // PostMultiplied or Inherit, which causes
                        // the FIRST presented frame to flash a
                        // garbage-colour (often purple/magenta) for
                        // a few ms while the OS compositor decides
                        // how to blend an uninitialized surface
                        // against whatever's behind the window.
                        // Opaque tells the compositor "draw my
                        // pixels verbatim, no alpha" — same shape
                        // ghostty / kitty / alacritty use on macOS.
                        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
                            wgpu::CompositeAlphaMode::Opaque
                        } else {
                            caps.alpha_modes[0]
                        };

                        let surface_config = wgpu::SurfaceConfiguration {
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            format,
                            width: self.width.max(1),
                            height: self.height.max(1),
                            present_mode,
                            // Frame latency 1 (not the wgpu default 2) for
                            // input-responsive interactive use. With 2 the
                            // swapchain can hold two frames in flight; the
                            // display shows N-2 then N-1 then N over
                            // consecutive vsyncs, which on a slow-response
                            // LCD reads as the cursor / cells trailing
                            // behind the user with a stream of past states.
                            // Ghostty / Alacritty / Kitty all default to 1
                            // for the same reason.
                            desired_maximum_frame_latency: 1,
                            alpha_mode,
                            view_formats: vec![],
                        };
                        surface.configure(&gpu.device, &surface_config);

                        // Force an immediate opaque clear of the
                        // freshly-configured surface so the first
                        // present is the operator's background colour,
                        // not whatever uninitialized pixels Metal
                        // happened to allocate. Without this the
                        // window briefly flashes a near-magenta
                        // value before mado's render loop ticks.
                        if let Ok(frame) = surface.get_current_texture() {
                            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                            let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("madori_first_clear"),
                            });
                            {
                                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("madori_first_clear_pass"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: &view,
                                        resolve_target: None,
                                        ops: wgpu::Operations {
                                            // Match mado's Nord default bg (#2e3440 linear).
                                            // Slightly different from the renderer's
                                            // bg_color (sRGB-corrected) but visually
                                            // indistinguishable; the goal is "not
                                            // magenta", not "exact theme color".
                                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                                r: 0.027, g: 0.035, b: 0.047, a: 1.0,
                                            }),
                                            store: wgpu::StoreOp::Store,
                                        },
                                    })],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                });
                            }
                            gpu.queue.submit(std::iter::once(encoder.finish()));
                            frame.present();
                        }

                        let text = garasu::TextRenderer::new(&gpu.device, &gpu.queue, format);

                        self.renderer.init(&gpu);
                        self.text = Some(text);
                        self.surface_config = Some(surface_config);
                        self.surface = Some(surface);
                        self.gpu = Some(gpu);
                    }
                    Err(e) => {
                        tracing::error!("GPU initialization failed: {e}");
                        event_loop.exit();
                        return;
                    }
                }

                self.window = Some(window);
            }

            fn window_event(
                &mut self,
                event_loop: &winit::event_loop::ActiveEventLoop,
                _window_id: winit::window::WindowId,
                event: WindowEvent,
            ) {
                match &event {
                    WindowEvent::CloseRequested => {
                        let app_event = AppEvent::CloseRequested;
                        let resp = self.dispatch(&app_event, event_loop);
                        if !resp.consumed {
                            event_loop.exit();
                        }
                    }
                    WindowEvent::Resized(size) => {
                        self.width = size.width.max(1);
                        self.height = size.height.max(1);
                        if let (Some(surface), Some(cfg), Some(gpu)) =
                            (&self.surface, &mut self.surface_config, &self.gpu)
                        {
                            cfg.width = self.width;
                            cfg.height = self.height;
                            surface.configure(&gpu.device, cfg);
                        }
                        self.renderer.resize(self.width, self.height);
                        let app_event = AppEvent::Resized {
                            width: self.width,
                            height: self.height,
                        };
                        self.dispatch(&app_event, event_loop);
                    }
                    WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        // Updates the cached scale-factor so the next
                        // RenderContext carries the fresh value. winit
                        // pairs every ScaleFactorChanged with a
                        // Resized in the same frame, which reconfigures
                        // the surface — we don't need to do that here.
                        self.scale_factor = *scale_factor;
                    }
                    WindowEvent::Focused(focused) => {
                        let app_event = AppEvent::Focused(*focused);
                        self.dispatch(&app_event, event_loop);
                    }
                    WindowEvent::ModifiersChanged(mods) => {
                        self.modifiers = mods.state();
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        let key_event = KeyEvent {
                            key: KeyCode::from_winit(&event.logical_key),
                            pressed: event.state == ElementState::Pressed,
                            modifiers: Modifiers::from_winit(&self.modifiers),
                            text: event.text.as_ref().map(|t| t.to_string()),
                        };
                        let app_event = AppEvent::Key(key_event);
                        self.dispatch(&app_event, event_loop);
                    }
                    WindowEvent::Ime(ime) => {
                        let ime_event = match ime {
                            winit::event::Ime::Enabled => ImeEvent::Enabled,
                            winit::event::Ime::Preedit(text, cursor) => {
                                ImeEvent::Preedit(text.clone(), *cursor)
                            }
                            winit::event::Ime::Commit(text) => ImeEvent::Commit(text.clone()),
                            winit::event::Ime::Disabled => ImeEvent::Disabled,
                        };
                        let app_event = AppEvent::Ime(ime_event);
                        self.dispatch(&app_event, event_loop);
                    }
                    WindowEvent::CursorMoved { position, .. } => {
                        self.cursor_x = position.x;
                        self.cursor_y = position.y;
                        let app_event = AppEvent::Mouse(MouseEvent::Moved {
                            x: position.x,
                            y: position.y,
                        });
                        self.dispatch(&app_event, event_loop);
                    }
                    WindowEvent::MouseInput { state, button, .. } => {
                        let btn = match button {
                            winit::event::MouseButton::Left => MouseButton::Left,
                            winit::event::MouseButton::Right => MouseButton::Right,
                            winit::event::MouseButton::Middle => MouseButton::Middle,
                            _ => MouseButton::Left,
                        };
                        let app_event = AppEvent::Mouse(MouseEvent::Button {
                            button: btn,
                            pressed: *state == ElementState::Pressed,
                            x: self.cursor_x,
                            y: self.cursor_y,
                            modifiers: Modifiers::from_winit(&self.modifiers),
                        });
                        self.dispatch(&app_event, event_loop);
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let (dx, dy) = match delta {
                            winit::event::MouseScrollDelta::LineDelta(x, y) => {
                                (f64::from(*x), f64::from(*y))
                            }
                            winit::event::MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                        };
                        let app_event = AppEvent::Mouse(MouseEvent::Scroll { dx, dy });
                        self.dispatch(&app_event, event_loop);
                    }
                    WindowEvent::RedrawRequested => {
                        // Dispatch redraw event to handler (for title updates, exit checks, etc.)
                        let redraw_event = AppEvent::RedrawRequested;
                        self.dispatch(&redraw_event, event_loop);

                        if let (Some(surface), Some(gpu), Some(text)) =
                            (&self.surface, &self.gpu, &mut self.text)
                        {
                            let frame = match surface.get_current_texture() {
                                Ok(f) => f,
                                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                                    if let Some(cfg) = &self.surface_config {
                                        surface.configure(&gpu.device, cfg);
                                    }
                                    return;
                                }
                                Err(e) => {
                                    tracing::warn!("surface error: {e}");
                                    return;
                                }
                            };
                            let view = frame
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());

                            let now = Instant::now();
                            let elapsed = now.duration_since(self.start_time).as_secs_f32();
                            let dt = now.duration_since(self.last_frame).as_secs_f32();
                            self.last_frame = now;

                            let mut render_ctx = RenderContext {
                                gpu,
                                text,
                                surface_view: &view,
                                width: self.width,
                                height: self.height,
                                scale_factor: self.scale_factor,
                                elapsed,
                                dt,
                            };
                            self.renderer.render(&mut render_ctx);

                            frame.present();

                            // First-frame reveal — show the window only
                            // after the swapchain has real pixels in it.
                            // Prevents the multicolor purple flash that
                            // results from showing a window whose
                            // backbuffer still holds uninitialised GPU
                            // memory.
                            if !self.first_frame_presented {
                                self.first_frame_presented = true;
                                if let Some(w) = &self.window {
                                    w.set_visible(true);
                                }
                            }
                        }
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                    _ => {}
                }
            }
        }

        let event_loop = EventLoop::new().map_err(|e| MadoriError::EventLoop(e.to_string()))?;
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

        let mut handler = Handler {
            config,
            renderer,
            event_handler,
            window: None,
            gpu: None,
            text: None,
            surface: None,
            surface_config: None,
            start_time: Instant::now(),
            last_frame: Instant::now(),
            modifiers: winit::keyboard::ModifiersState::default(),
            width: 0,
            height: 0,
            // 1.0 is the safe pre-resume default — gets overwritten by
            // `window.scale_factor()` in `resumed` before the first
            // render fires.
            scale_factor: 1.0,
            cursor_x: 0.0,
            cursor_y: 0.0,
            first_frame_presented: false,
        };

        event_loop
            .run_app(&mut handler)
            .map_err(|e| MadoriError::EventLoop(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_config_defaults() {
        let config = AppConfig::default();
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
        assert!(config.resizable);
        assert!(config.vsync);
        assert!(!config.transparent);
    }

    #[test]
    fn builder_fluent_api() {
        use crate::render::ClearRenderer;
        let builder = App::builder(ClearRenderer::default())
            .title("Test")
            .size(800, 600);
        assert_eq!(builder.config.title, "Test");
        assert_eq!(builder.config.width, 800);
        assert_eq!(builder.config.height, 600);
    }

    #[test]
    fn event_response_from_bool() {
        let resp: EventResponse = true.into();
        assert!(resp.consumed);
        assert!(!resp.exit);
        assert!(resp.set_title.is_none());

        let resp: EventResponse = false.into();
        assert!(!resp.consumed);
    }
}
