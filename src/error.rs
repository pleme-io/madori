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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn gpu_init_display() {
        let e = MadoriError::GpuInit("no adapter".into());
        assert_eq!(e.to_string(), "GPU initialization failed: no adapter");
    }

    #[test]
    fn window_display() {
        let e = MadoriError::Window("no display".into());
        assert_eq!(e.to_string(), "window creation failed: no display");
    }

    #[test]
    fn render_display() {
        let e = MadoriError::Render("surface lost".into());
        assert_eq!(e.to_string(), "render error: surface lost");
    }

    #[test]
    fn event_loop_display() {
        let e = MadoriError::EventLoop("killed".into());
        assert_eq!(e.to_string(), "event loop error: killed");
    }

    #[test]
    fn garasu_from_conversion_preserves_source() {
        // `#[from] garasu::GarasuError` means `?` converts a garasu error
        // into MadoriError::Garasu without losing the chain. If a future
        // refactor drops #[from] (e.g. Box<dyn Error>), Error::source()
        // would return None here and downstream diagnostics would break.
        let inner = garasu::GarasuError::Gpu("adapter missing".into());
        let outer: MadoriError = inner.into();
        match &outer {
            MadoriError::Garasu(_) => {}
            other => panic!("expected Garasu variant, got {other:?}"),
        }
        assert!(outer.source().is_some());
        assert_eq!(outer.to_string(), "garasu error: GPU error: adapter missing");
    }

    #[test]
    fn result_alias_is_std_result() {
        // Result<T> is `std::result::Result<T, MadoriError>`. If this
        // alias ever shadows to anyhow::Result or similar, `?` chains
        // throughout the crate would change their error semantics.
        fn ok() -> Result<u8> { Ok(7) }
        fn err() -> Result<u8> { Err(MadoriError::Render("x".into())) }
        assert_eq!(ok().unwrap(), 7);
        assert!(err().is_err());
    }

    #[test]
    fn variants_are_distinguishable() {
        // Each variant matches its own arm — guards against an accidental
        // `Box<dyn Error>` collapse that would merge variants.
        let cases: Vec<MadoriError> = vec![
            MadoriError::GpuInit("a".into()),
            MadoriError::Window("b".into()),
            MadoriError::Render("c".into()),
            MadoriError::EventLoop("d".into()),
        ];
        for case in cases {
            match case {
                MadoriError::GpuInit(s) => assert_eq!(s, "a"),
                MadoriError::Window(s) => assert_eq!(s, "b"),
                MadoriError::Render(s) => assert_eq!(s, "c"),
                MadoriError::EventLoop(s) => assert_eq!(s, "d"),
                MadoriError::Garasu(_) => panic!("Garasu matched non-Garasu"),
            }
        }
    }
}
