pub mod app;
mod doorbell;
pub mod error;
pub mod event;
#[cfg(target_os = "macos")]
mod metal_layer;
mod pacer;
pub mod render;

pub use app::{App, AppBuilder, AppConfig, FramePacing, LoopClosed, MenuPolicy, UserProxy};
pub use error::MadoriError;
pub use event::{
    AppEvent, EventResponse, ImeEvent, InputEvent, KeyEvent, Modifiers, MouseEvent, PointerIcon,
    ScrollDelta,
};
pub use pacer::{Surface, Visibility, Visible};
pub use render::{FrameDemand, FrameQuery, RenderCallback, RenderContext};
