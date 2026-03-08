#[derive(thiserror::Error, Debug)]
pub enum MadoriError {
    #[error("GPU initialization failed: {0}")]
    GpuInit(String),

    #[error("window creation failed: {0}")]
    Window(String),

    #[error("render error: {0}")]
    Render(String),

    #[error("garasu error: {0}")]
    Garasu(#[from] garasu::GarasuError),

    #[error("event loop error: {0}")]
    EventLoop(String),
}

pub type Result<T> = std::result::Result<T, MadoriError>;
