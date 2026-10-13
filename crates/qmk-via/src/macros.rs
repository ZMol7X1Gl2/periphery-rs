//! VIA/QMK dynamic macros. The buffer holds NUL-terminated macros; printable
//! ASCII is typed as-is and `0x01` introduces an action:
//! `01 01 kc` tap, `01 02 kc` press, `01 03 kc` release, `01 04 <digits> |` delay.
//!
//! Text form used by the UI: plain text plus `{Key}` tap, `{+Key}` press,
//! `{-Key}` release and `{250}` delay in ms (key names = keymap labels).

use anyhow::{Result, bail};

const PREFIX: u8 = 0x01;
const TAP: u8 = 0x01;
const DOWN: u8 = 0x02;
const UP: u8 = 0x03;
const DELAY: u8 = 0x04;

/// Splits the raw buffer into `count` macros of raw bytes.
pub fn split(buf: &[u8], count: usize) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = buf.split(|b| *b == 0).take(count).map(<[u8]>::to_vec).collect();
    out.resize(count, Vec::new());
    out
}

pub fn to_text(m: &[u8], label: &dyn Fn(u16) -> String) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i < m.len() {
        if m[i] == PREFIX && i + 1 < m.len() {
            match m[i + 1] {
                TAP | DOWN | UP if i + 2 < m.len() => {
                    let sign = match m[i + 1] {
                        DOWN => "+",
                        UP => "-",
                        _ => "",
                    };
                    s.push_str(&format!("{{{sign}{}}}", label(m[i + 2] as u16)));
                    i += 3;
                    continue;
                }
                DELAY => {
                    let end = m[i + 2..].iter().position(|b| *b == b'|').map_or(m.len(), |p| i + 2 + p);
                    s.push_str(&format!("{{{}}}", String::from_utf8_lossy(&m[i + 2..end])));
                    i = end + 1;
                    continue;
                }
                _ => {}
            }
        }
        match m[i] {
            b'\n' => s.push_str("{Enter}"),
            b'\t' => s.push_str("{Tab}"),
            c if c.is_ascii_graphic() || c == b' ' => s.push(c as char),
            c => s.push_str(&format!("{{{c:#04x}}}")),
        }
        i += 1;
    }
    s
}

pub fn from_text(text: &str, code_for: &dyn Fn(&str) -> Option<u16>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '{' {
            let Some(end) = rest.find('}') else { bail!("missing '}}' in macro") };
            let token = rest[1..end].trim();
            rest = &rest[end + 1..];
            if !token.is_empty() && token.chars().all(|c| c.is_ascii_digit()) {
                out.extend([PREFIX, DELAY]);
                out.extend(token.as_bytes());
                out.push(b'|');
                continue;
            }
            let (action, name) = match token.as_bytes().first() {
                Some(b'+') => (DOWN, &token[1..]),
                Some(b'-') if token.len() > 1 => (UP, &token[1..]),
                _ => (TAP, token),
            };
            let Some(code) = code_for(name) else { bail!("unknown key {{{name}}} in macro") };
            if code > 0xFF {
                bail!("{{{name}}} can't be used in a macro (only basic keys)");
            }
            out.extend([PREFIX, action, code as u8]);
        } else if c.is_ascii() && (c.is_ascii_graphic() || c == ' ') {
            out.push(c as u8);
            rest = &rest[1..];
        } else {
            bail!("macros can only type plain ASCII characters (found {c:?})");
        }
    }
    Ok(out)
}

/// Joins macros into a buffer of exactly `size` bytes.
pub fn join(macros: &[Vec<u8>], size: usize) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::with_capacity(size);
    for m in macros {
        buf.extend(m);
        buf.push(0);
    }
    if buf.len() > size {
        bail!("macros use {} bytes but the keyboard only has {size}", buf.len());
    }
    buf.resize(size, 0);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let label = |c: u16| match c {
            0xE0 => "LCtrl".to_string(),
            0x28 => "Enter".to_string(),
            0x06 => "C".to_string(),
            c => format!("{c:#x}"),
        };
        let code = |n: &str| match n {
            "LCtrl" => Some(0xE0),
            "Enter" => Some(0x28),
            "C" => Some(0x06),
            _ => None,
        };
        let text = "Hi there{Enter}{+LCtrl}{C}{-LCtrl}{250}ok";
        let bytes = from_text(text, &code).unwrap();
        assert_eq!(&bytes[..8], b"Hi there");
        assert_eq!(&bytes[8..11], &[1, 1, 0x28]);
        assert_eq!(to_text(&bytes, &label), text);
        assert!(from_text("{Nope}", &code).is_err());
        let buf = join(&[bytes.clone(), Vec::new(), b"x".to_vec()], 64).unwrap();
        assert_eq!(split(&buf, 4)[0], bytes);
        assert_eq!(split(&buf, 4)[2], b"x");
    }
}
