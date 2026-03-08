use serde::{Deserialize, Serialize};

/// Top-level application event.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Window resized to new physical dimensions.
    Resized { width: u32, height: u32 },
    /// Window close requested.
    CloseRequested,
    /// Window gained or lost focus.
    Focused(bool),
    /// Keyboard input.
    Key(KeyEvent),
    /// Mouse input.
    Mouse(MouseEvent),
    /// IME pre-edit (composition) text update.
    Ime(ImeEvent),
    /// Redraw requested (vsync tick or explicit).
    RedrawRequested,
}

/// Actions the event handler can request from the framework.
#[derive(Debug, Clone, Default)]
pub struct EventResponse {
    /// Whether the event was consumed (prevents default handling).
    pub consumed: bool,
    /// Request the event loop to exit.
    pub exit: bool,
    /// Request a window title change.
    pub set_title: Option<String>,
}

impl EventResponse {
    /// Create a response that consumes the event.
    #[must_use]
    pub fn consumed() -> Self {
        Self { consumed: true, ..Default::default() }
    }

    /// Create a response that does not consume the event.
    #[must_use]
    pub fn ignored() -> Self {
        Self::default()
    }
}

impl From<bool> for EventResponse {
    fn from(consumed: bool) -> Self {
        Self { consumed, ..Default::default() }
    }
}

/// IME (Input Method Editor) event for CJK/compose input.
#[derive(Debug, Clone)]
pub enum ImeEvent {
    /// IME is enabled.
    Enabled,
    /// Pre-edit text while composing (with optional cursor position).
    Preedit(String, Option<(usize, usize)>),
    /// Final committed text.
    Commit(String),
    /// IME is disabled.
    Disabled,
}

/// Keyboard event (press or release).
#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub key: KeyCode,
    pub pressed: bool,
    pub modifiers: Modifiers,
    /// Text input if this produced a character.
    pub text: Option<String>,
}

/// Mouse event.
#[derive(Debug, Clone)]
pub enum MouseEvent {
    Moved { x: f64, y: f64 },
    Button { button: MouseButton, pressed: bool, x: f64, y: f64 },
    Scroll { dx: f64, dy: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Active modifier keys.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Platform-independent key codes for common keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
    Space,
    /// Key not mapped to any known code.
    Unknown,
}

/// Convert winit keyboard events to our KeyCode.
impl KeyCode {
    #[must_use]
    pub fn from_winit(key: &winit::keyboard::Key) -> Self {
        use winit::keyboard::{Key as WKey, NamedKey};
        match key {
            WKey::Named(named) => match named {
                NamedKey::Enter => Self::Enter,
                NamedKey::Escape => Self::Escape,
                NamedKey::Backspace => Self::Backspace,
                NamedKey::Delete => Self::Delete,
                NamedKey::Tab => Self::Tab,
                NamedKey::ArrowUp => Self::Up,
                NamedKey::ArrowDown => Self::Down,
                NamedKey::ArrowLeft => Self::Left,
                NamedKey::ArrowRight => Self::Right,
                NamedKey::Home => Self::Home,
                NamedKey::End => Self::End,
                NamedKey::PageUp => Self::PageUp,
                NamedKey::PageDown => Self::PageDown,
                NamedKey::Space => Self::Space,
                NamedKey::F1 => Self::F(1),
                NamedKey::F2 => Self::F(2),
                NamedKey::F3 => Self::F(3),
                NamedKey::F4 => Self::F(4),
                NamedKey::F5 => Self::F(5),
                NamedKey::F6 => Self::F(6),
                NamedKey::F7 => Self::F(7),
                NamedKey::F8 => Self::F(8),
                NamedKey::F9 => Self::F(9),
                NamedKey::F10 => Self::F(10),
                NamedKey::F11 => Self::F(11),
                NamedKey::F12 => Self::F(12),
                _ => Self::Unknown,
            },
            WKey::Character(c) => {
                let mut chars = c.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) => Self::Char(ch),
                    _ => Self::Unknown,
                }
            }
            _ => Self::Unknown,
        }
    }
}

impl Modifiers {
    #[must_use]
    pub fn from_winit(state: &winit::keyboard::ModifiersState) -> Self {
        Self {
            shift: state.shift_key(),
            ctrl: state.control_key(),
            alt: state.alt_key(),
            meta: state.super_key(),
        }
    }

    #[must_use]
    pub fn any(&self) -> bool {
        self.shift || self.ctrl || self.alt || self.meta
    }
}

/// Input event combining key + mouse for consumers.
#[derive(Debug, Clone)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_any() {
        let none = Modifiers::default();
        assert!(!none.any());

        let shift = Modifiers { shift: true, ..Default::default() };
        assert!(shift.any());
    }

    #[test]
    fn key_code_char() {
        let k = KeyCode::Char('a');
        assert_eq!(k, KeyCode::Char('a'));
        assert_ne!(k, KeyCode::Char('b'));
    }
}
