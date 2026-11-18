//! Linux evdev key and button codes (`linux/input-event-codes.h`) to Spawn
//! identifiers. Wayland and X11 both report evdev codes (X11 offset by 8).

use crate::key::{KeyCode, MouseButton};

/// Maps an evdev `KEY_*` code to a physical key.
pub(crate) fn key_code(code: u32) -> KeyCode {
    use KeyCode as K;
    match code {
        1 => K::Escape,
        2 => K::Digit1,
        3 => K::Digit2,
        4 => K::Digit3,
        5 => K::Digit4,
        6 => K::Digit5,
        7 => K::Digit6,
        8 => K::Digit7,
        9 => K::Digit8,
        10 => K::Digit9,
        11 => K::Digit0,
        12 => K::Minus,
        13 => K::Equal,
        14 => K::Backspace,
        15 => K::Tab,
        16 => K::Q,
        17 => K::W,
        18 => K::E,
        19 => K::R,
        20 => K::T,
        21 => K::Y,
        22 => K::U,
        23 => K::I,
        24 => K::O,
        25 => K::P,
        26 => K::BracketLeft,
        27 => K::BracketRight,
        28 => K::Enter,
        29 => K::ControlLeft,
        30 => K::A,
        31 => K::S,
        32 => K::D,
        33 => K::F,
        34 => K::G,
        35 => K::H,
        36 => K::J,
        37 => K::K,
        38 => K::L,
        39 => K::Semicolon,
        40 => K::Quote,
        41 => K::Backquote,
        42 => K::ShiftLeft,
        43 => K::Backslash,
        44 => K::Z,
        45 => K::X,
        46 => K::C,
        47 => K::V,
        48 => K::B,
        49 => K::N,
        50 => K::M,
        51 => K::Comma,
        52 => K::Period,
        53 => K::Slash,
        54 => K::ShiftRight,
        55 => K::NumpadMultiply,
        56 => K::AltLeft,
        57 => K::Space,
        58 => K::CapsLock,
        59 => K::F1,
        60 => K::F2,
        61 => K::F3,
        62 => K::F4,
        63 => K::F5,
        64 => K::F6,
        65 => K::F7,
        66 => K::F8,
        67 => K::F9,
        68 => K::F10,
        69 => K::NumLock,
        70 => K::ScrollLock,
        71 => K::Numpad7,
        72 => K::Numpad8,
        73 => K::Numpad9,
        74 => K::NumpadSubtract,
        75 => K::Numpad4,
        76 => K::Numpad5,
        77 => K::Numpad6,
        78 => K::NumpadAdd,
        79 => K::Numpad1,
        80 => K::Numpad2,
        81 => K::Numpad3,
        82 => K::Numpad0,
        83 => K::NumpadDecimal,
        86 => K::IntlBackslash,
        87 => K::F11,
        88 => K::F12,
        96 => K::NumpadEnter,
        97 => K::ControlRight,
        98 => K::NumpadDivide,
        99 => K::PrintScreen,
        100 => K::AltRight,
        102 => K::Home,
        103 => K::ArrowUp,
        104 => K::PageUp,
        105 => K::ArrowLeft,
        106 => K::ArrowRight,
        107 => K::End,
        108 => K::ArrowDown,
        109 => K::PageDown,
        110 => K::Insert,
        111 => K::Delete,
        119 => K::Pause,
        125 => K::SuperLeft,
        126 => K::SuperRight,
        127 => K::ContextMenu,
        183 => K::F13,
        184 => K::F14,
        185 => K::F15,
        186 => K::F16,
        187 => K::F17,
        188 => K::F18,
        189 => K::F19,
        190 => K::F20,
        191 => K::F21,
        192 => K::F22,
        193 => K::F23,
        194 => K::F24,
        _ => K::Unidentified,
    }
}

/// Maps an evdev `BTN_*` code to a pointer button.
pub(crate) fn mouse_button(code: u32) -> MouseButton {
    match code {
        0x110 => MouseButton::Left,
        0x111 => MouseButton::Right,
        0x112 => MouseButton::Middle,
        // BTN_SIDE and BTN_BACK are both reported for "back" side buttons.
        0x113 | 0x116 => MouseButton::Back,
        // BTN_EXTRA and BTN_FORWARD are both reported for "forward" side buttons.
        0x114 | 0x115 => MouseButton::Forward,
        other => MouseButton::Other(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_movement_cluster_by_position() {
        // KEY_W, KEY_A, KEY_S, KEY_D: physical positions, independent of layout.
        assert_eq!(key_code(17), KeyCode::W);
        assert_eq!(key_code(30), KeyCode::A);
        assert_eq!(key_code(31), KeyCode::S);
        assert_eq!(key_code(32), KeyCode::D);
    }

    #[test]
    fn maps_function_and_navigation_keys() {
        assert_eq!(key_code(1), KeyCode::Escape);
        assert_eq!(key_code(59), KeyCode::F1);
        assert_eq!(key_code(88), KeyCode::F12);
        assert_eq!(key_code(194), KeyCode::F24);
        assert_eq!(key_code(103), KeyCode::ArrowUp);
        assert_eq!(key_code(125), KeyCode::SuperLeft);
    }

    #[test]
    fn unknown_key_codes_are_unidentified() {
        assert_eq!(key_code(0), KeyCode::Unidentified);
        assert_eq!(key_code(84), KeyCode::Unidentified);
        assert_eq!(key_code(u32::MAX), KeyCode::Unidentified);
    }

    #[test]
    fn no_two_codes_map_to_the_same_named_key() {
        let mut seen = std::collections::HashMap::new();
        for code in 0..=255 {
            let key = key_code(code);
            if key != KeyCode::Unidentified {
                assert_eq!(seen.insert(key, code), None, "{key:?} mapped twice");
            }
        }
    }

    #[test]
    fn maps_mouse_buttons() {
        assert_eq!(mouse_button(0x110), MouseButton::Left);
        assert_eq!(mouse_button(0x111), MouseButton::Right);
        assert_eq!(mouse_button(0x112), MouseButton::Middle);
        assert_eq!(mouse_button(0x113), MouseButton::Back);
        assert_eq!(mouse_button(0x114), MouseButton::Forward);
        assert_eq!(mouse_button(0x117), MouseButton::Other(0x117));
    }
}
