//! Keybinding strings like `"win+shift+h"`, parsed into key chords.

use std::fmt;
use std::str::FromStr;

/// A modifier set plus one Windows virtual-key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Windows virtual-key code (`VK_*`).
    pub vk: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("empty key binding")]
    Empty,
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("`{0}` has no key, only modifiers")]
    NoKey(String),
    #[error("`{0}` has more than one non-modifier key")]
    TwoKeys(String),
}

/// Named keys and their virtual-key codes. Letters and digits are handled
/// separately.
const NAMED: &[(&str, u16)] = &[
    ("enter", 0x0D),
    ("return", 0x0D),
    ("space", 0x20),
    ("tab", 0x09),
    ("escape", 0x1B),
    ("esc", 0x1B),
    ("backspace", 0x08),
    ("delete", 0x2E),
    ("insert", 0x2D),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("left", 0x25),
    ("up", 0x26),
    ("right", 0x27),
    ("down", 0x28),
    ("grave", 0xC0),
    ("backtick", 0xC0),
    ("minus", 0xBD),
    ("equal", 0xBB),
    ("plus", 0xBB),
    ("comma", 0xBC),
    ("period", 0xBE),
    ("slash", 0xBF),
    ("semicolon", 0xBA),
    ("quote", 0xDE),
    ("lbracket", 0xDB),
    ("rbracket", 0xDD),
    ("backslash", 0xDC),
];

fn key_code(name: &str) -> Option<u16> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_alphanumeric()) {
        return Some(bytes[0].to_ascii_uppercase() as u16);
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    NAMED.iter().find(|(k, _)| *k == name).map(|(_, vk)| *vk)
}

fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => ((vk as u8) as char).to_ascii_lowercase().to_string(),
        0x70..=0x87 => format!("f{}", vk - 0x70 + 1),
        _ => NAMED
            .iter()
            .find(|(_, v)| *v == vk)
            .map_or_else(|| format!("vk{vk:#04x}"), |(k, _)| (*k).to_string()),
    }
}

impl FromStr for Chord {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return Err(KeyError::Empty);
        }
        let mut chord = Chord {
            win: false,
            ctrl: false,
            alt: false,
            shift: false,
            vk: 0,
        };
        let mut key = None;
        for part in lower.split('+').map(str::trim) {
            match part {
                "win" | "super" | "mod" => chord.win = true,
                "ctrl" | "control" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                other => {
                    let vk = key_code(other).ok_or_else(|| KeyError::UnknownKey(other.into()))?;
                    if key.replace(vk).is_some() {
                        return Err(KeyError::TwoKeys(s.into()));
                    }
                }
            }
        }
        chord.vk = key.ok_or_else(|| KeyError::NoKey(s.into()))?;
        Ok(chord)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.win, "win+"),
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(&key_name(self.vk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn parses_modifiers_and_keys() {
        let c = chord("Win+Shift+H");
        assert!(c.win && c.shift && !c.ctrl && !c.alt);
        assert_eq!(c.vk, 0x48);
        assert_eq!(chord("win+enter").vk, 0x0D);
        assert_eq!(chord("win+1").vk, 0x31);
        assert_eq!(chord("win+grave").vk, 0xC0);
        assert_eq!(chord("alt+f4").vk, 0x73);
        assert_eq!(chord("super + space"), chord("win+space"));
    }

    #[test]
    fn round_trips_through_display() {
        for s in [
            "win+shift+h",
            "win+enter",
            "ctrl+alt+f12",
            "win+grave",
            "win+5",
        ] {
            assert_eq!(chord(s).to_string(), s);
        }
    }

    #[test]
    fn rejects_bad_bindings() {
        assert_eq!("".parse::<Chord>(), Err(KeyError::Empty));
        assert!(matches!(
            "win+shift".parse::<Chord>(),
            Err(KeyError::NoKey(_))
        ));
        assert!(matches!(
            "win+a+b".parse::<Chord>(),
            Err(KeyError::TwoKeys(_))
        ));
        assert!(matches!(
            "win+banana".parse::<Chord>(),
            Err(KeyError::UnknownKey(_))
        ));
        assert!(matches!(
            "win+f25".parse::<Chord>(),
            Err(KeyError::UnknownKey(_))
        ));
    }
}
