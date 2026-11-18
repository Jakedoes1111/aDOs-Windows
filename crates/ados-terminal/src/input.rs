use alacritty_terminal::term::TermMode;
use gpui_kit::Keystroke;

/// Legacy xterm encoding, including application cursor and modified navigation.
pub fn encode(key: &Keystroke, mode: TermMode) -> Vec<u8> {
    let m = key.modifiers;
    if m.platform {
        return Vec::new();
    }
    let modifier = 1 + usize::from(m.shift) + 2 * usize::from(m.alt) + 4 * usize::from(m.control);
    let cursor = match key.key.as_str() {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(suffix) = cursor {
        return if modifier != 1 {
            format!("\x1b[1;{modifier}{suffix}")
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{suffix}")
        } else {
            format!("\x1b[{suffix}")
        }
        .into_bytes();
    }
    let tilde = match key.key.as_str() {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    };
    if let Some(code) = tilde {
        return if modifier == 1 {
            format!("\x1b[{code}~")
        } else {
            format!("\x1b[{code};{modifier}~")
        }
        .into_bytes();
    }
    if let Some(suffix) = match key.key.as_str() {
        "f1" => Some('P'),
        "f2" => Some('Q'),
        "f3" => Some('R'),
        "f4" => Some('S'),
        _ => None,
    } {
        return if modifier == 1 {
            format!("\x1bO{suffix}")
        } else {
            format!("\x1b[1;{modifier}{suffix}")
        }
        .into_bytes();
    }
    let mut bytes = match key.key.as_str() {
        "enter" => {
            if mode.contains(TermMode::LINE_FEED_NEW_LINE) {
                b"\r\n".to_vec()
            } else {
                vec![b'\r']
            }
        }
        "escape" => vec![0x1b],
        "backspace" => vec![if m.control { 0x08 } else { 0x7f }],
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        _ if m.control => {
            let name = key.key.as_str();
            let c = match name {
                "space" | "@" | "2" => Some(0),
                "[" | "3" => Some(27),
                "\\" | "4" => Some(28),
                "]" | "5" => Some(29),
                "^" | "6" => Some(30),
                "_" | "-" | "/" | "7" => Some(31),
                "?" | "8" => Some(127),
                _ if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() => {
                    Some(name.as_bytes()[0].to_ascii_uppercase() & 0x1f)
                }
                _ => None,
            };
            c.into_iter().collect()
        }
        "space" => vec![b' '],
        _ => key
            .key_char
            .as_deref()
            .or_else(|| (key.key.chars().count() == 1).then_some(key.key.as_str()))
            .unwrap_or("")
            .as_bytes()
            .to_vec(),
    };
    if m.alt && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // Never let clipboard content terminate the bracket and execute a command.
        format!(
            "\x1b[200~{}\x1b[201~",
            text.replace('\x1b', "").replace('\u{009b}', "")
        )
        .into_bytes()
    } else {
        text.replace("\r\n", "\n").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_and_control() {
        assert_eq!(
            encode(
                &Keystroke::parse("ctrl-c").expect("test key"),
                TermMode::empty()
            ),
            b"\x03"
        );
        assert_eq!(
            encode(
                &Keystroke::parse("alt-x").expect("test key"),
                TermMode::empty()
            ),
            b"\x1bx"
        );
        assert_eq!(
            encode(
                &Keystroke::parse("up").expect("test key"),
                TermMode::APP_CURSOR
            ),
            b"\x1bOA"
        );
        assert_eq!(
            encode(
                &Keystroke::parse("ctrl-right").expect("test key"),
                TermMode::empty()
            ),
            b"\x1b[1;5C"
        );
    }
    #[test]
    fn paste_cannot_escape_brackets() {
        assert_eq!(paste("a\x1b[201~b", true), b"\x1b[200~a[201~b\x1b[201~");
        assert_eq!(paste("a\r\nb", false), b"a\rb");
    }
}
