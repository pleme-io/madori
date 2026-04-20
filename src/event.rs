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
    /// Request fullscreen toggle.
    pub toggle_fullscreen: bool,
    /// Request cursor visibility change (None = no change).
    pub set_cursor_visible: Option<bool>,
}

impl EventResponse {
    /// Create a response that consumes the event.
    #[must_use]
    pub fn consumed() -> Self {
        Self {
            consumed: true,
            ..Default::default()
        }
    }

    /// Create a response that does not consume the event.
    #[must_use]
    pub fn ignored() -> Self {
        Self::default()
    }
}

impl From<bool> for EventResponse {
    fn from(consumed: bool) -> Self {
        Self {
            consumed,
            ..Default::default()
        }
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
    Moved {
        x: f64,
        y: f64,
    },
    Button {
        button: MouseButton,
        pressed: bool,
        x: f64,
        y: f64,
        modifiers: Modifiers,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
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
    use winit::keyboard::{Key as WKey, NamedKey};

    #[test]
    fn modifiers_any() {
        let none = Modifiers::default();
        assert!(!none.any());

        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        assert!(shift.any());
    }

    #[test]
    fn modifiers_any_covers_every_field() {
        // `any()` must short-circuit on each of the four fields. If a
        // future refactor (e.g. bitflags) drops a field from the OR, that
        // modifier would silently stop gating hotkeys — hard to notice.
        for modifier in [
            Modifiers { shift: true, ..Default::default() },
            Modifiers { ctrl: true, ..Default::default() },
            Modifiers { alt: true, ..Default::default() },
            Modifiers { meta: true, ..Default::default() },
        ] {
            assert!(modifier.any(), "any() false for {modifier:?}");
        }
    }

    #[test]
    fn modifiers_any_with_combinations() {
        let all = Modifiers { shift: true, ctrl: true, alt: true, meta: true };
        assert!(all.any());
        let ctrl_alt = Modifiers { ctrl: true, alt: true, ..Default::default() };
        assert!(ctrl_alt.any());
    }

    #[test]
    fn modifiers_serde_roundtrip() {
        // Modifiers participates in config serialization (hotkeys stored
        // as YAML). A rename or reorder would silently break user
        // configs — pin the field set via JSON round-trip.
        let original = Modifiers { shift: true, ctrl: false, alt: true, meta: false };
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"shift\":true"));
        assert!(json.contains("\"alt\":true"));
        let back: Modifiers = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shift, original.shift);
        assert_eq!(back.ctrl, original.ctrl);
        assert_eq!(back.alt, original.alt);
        assert_eq!(back.meta, original.meta);
    }

    #[test]
    fn key_code_char() {
        let k = KeyCode::Char('a');
        assert_eq!(k, KeyCode::Char('a'));
        assert_ne!(k, KeyCode::Char('b'));
    }

    #[test]
    fn key_code_from_winit_named_keys() {
        // Exhaustive table — if a NamedKey arm gets removed or
        // re-ordered silently (e.g. someone deletes ArrowUp thinking
        // it's unused), the paired app will stop responding to that key
        // and the user has no clue why.
        let cases: &[(WKey, KeyCode)] = &[
            (WKey::Named(NamedKey::Enter), KeyCode::Enter),
            (WKey::Named(NamedKey::Escape), KeyCode::Escape),
            (WKey::Named(NamedKey::Backspace), KeyCode::Backspace),
            (WKey::Named(NamedKey::Delete), KeyCode::Delete),
            (WKey::Named(NamedKey::Tab), KeyCode::Tab),
            (WKey::Named(NamedKey::ArrowUp), KeyCode::Up),
            (WKey::Named(NamedKey::ArrowDown), KeyCode::Down),
            (WKey::Named(NamedKey::ArrowLeft), KeyCode::Left),
            (WKey::Named(NamedKey::ArrowRight), KeyCode::Right),
            (WKey::Named(NamedKey::Home), KeyCode::Home),
            (WKey::Named(NamedKey::End), KeyCode::End),
            (WKey::Named(NamedKey::PageUp), KeyCode::PageUp),
            (WKey::Named(NamedKey::PageDown), KeyCode::PageDown),
            (WKey::Named(NamedKey::Space), KeyCode::Space),
        ];
        for (input, expected) in cases {
            assert_eq!(KeyCode::from_winit(input), *expected, "for {input:?}");
        }
    }

    #[test]
    fn key_code_from_winit_function_keys() {
        // F1..F12 all map to the same KeyCode::F(n) shape. Consumers
        // pattern-match `F(1)..=F(12)` so the numeric payload must be
        // exact; a typo like `NamedKey::F3 => Self::F(2)` would route
        // shortcuts to the wrong slot.
        let fkeys = [
            (NamedKey::F1, 1u8), (NamedKey::F2, 2), (NamedKey::F3, 3),
            (NamedKey::F4, 4), (NamedKey::F5, 5), (NamedKey::F6, 6),
            (NamedKey::F7, 7), (NamedKey::F8, 8), (NamedKey::F9, 9),
            (NamedKey::F10, 10), (NamedKey::F11, 11), (NamedKey::F12, 12),
        ];
        for (named, n) in fkeys {
            let got = KeyCode::from_winit(&WKey::Named(named));
            assert_eq!(got, KeyCode::F(n), "F-key mapped wrong: {named:?}");
        }
    }

    #[test]
    fn key_code_from_winit_character_single() {
        // `WKey::Character` with a single-code-point string becomes
        // `KeyCode::Char`. The parser takes two `chars.next()` calls to
        // assert there is no second char — regress this and we'd accept
        // multi-char strings as single chars.
        let k: winit::keyboard::SmolStr = "q".into();
        let got = KeyCode::from_winit(&WKey::Character(k));
        assert_eq!(got, KeyCode::Char('q'));
    }

    #[test]
    fn key_code_from_winit_character_multi_char() {
        // Dead-key compositions and some IME events produce
        // multi-char strings. We must map those to Unknown, not panic
        // and not take the first char silently (would lose input).
        let k: winit::keyboard::SmolStr = "ab".into();
        let got = KeyCode::from_winit(&WKey::Character(k));
        assert_eq!(got, KeyCode::Unknown);
    }

    #[test]
    fn key_code_from_winit_character_empty() {
        // Empty SmolStr — shouldn't occur in practice but the match arm
        // (None, None) needs to resolve to Unknown, not some default.
        let k: winit::keyboard::SmolStr = "".into();
        let got = KeyCode::from_winit(&WKey::Character(k));
        assert_eq!(got, KeyCode::Unknown);
    }

    #[test]
    fn key_code_from_winit_unmapped_named_is_unknown() {
        // Unmapped NamedKey variants fall into the catch-all `_ =>
        // Unknown` arm. CapsLock is intentionally unmapped — if someone
        // adds it without updating this test, we'll see the new mapping.
        let got = KeyCode::from_winit(&WKey::Named(NamedKey::CapsLock));
        assert_eq!(got, KeyCode::Unknown);
    }

    #[test]
    fn event_response_consumed_sets_only_consumed() {
        let r = EventResponse::consumed();
        assert!(r.consumed);
        assert!(!r.exit);
        assert!(r.set_title.is_none());
        assert!(!r.toggle_fullscreen);
        assert!(r.set_cursor_visible.is_none());
    }

    #[test]
    fn event_response_ignored_is_default() {
        let r = EventResponse::ignored();
        assert!(!r.consumed);
        assert!(!r.exit);
        assert!(r.set_title.is_none());
        assert!(!r.toggle_fullscreen);
        assert!(r.set_cursor_visible.is_none());
    }

    #[test]
    fn event_response_from_bool() {
        // `From<bool>` lets handlers write `return true.into()` or
        // similar. If someone drops the impl, ergonomic call sites
        // elsewhere stop compiling — this pins the shape.
        let consumed: EventResponse = true.into();
        assert!(consumed.consumed);
        assert!(!consumed.exit);

        let ignored: EventResponse = false.into();
        assert!(!ignored.consumed);
    }

    #[test]
    fn event_response_default_matches_ignored() {
        // Default and ignored() are intentionally identical — used at
        // thousands of call sites as `EventResponse::default()`. A
        // future refactor that changes one without the other would
        // produce subtly wrong handlers.
        let d = EventResponse::default();
        let i = EventResponse::ignored();
        assert_eq!(d.consumed, i.consumed);
        assert_eq!(d.exit, i.exit);
        assert_eq!(d.set_title, i.set_title);
        assert_eq!(d.toggle_fullscreen, i.toggle_fullscreen);
        assert_eq!(d.set_cursor_visible, i.set_cursor_visible);
    }

    #[test]
    fn modifiers_default_is_all_false() {
        // Modifiers::default() is used as the starting point for every
        // synthetic KeyEvent in tests + replay infra. All four fields
        // must start false or synthetic events arrive with phantom
        // modifiers set.
        let m = Modifiers::default();
        assert!(!m.shift);
        assert!(!m.ctrl);
        assert!(!m.alt);
        assert!(!m.meta);
    }
}
