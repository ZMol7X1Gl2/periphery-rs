//! VIA v3 "menus" (custom UI): each control maps to a (channel, value id)
//! pair read with `0x08` and written with `0x07`, persisted with `0x09`.

use serde_json::Value as Json;

#[derive(Debug, Clone)]
pub enum ItemKind {
    Range { min: i64, max: i64 },
    Dropdown(Vec<(i64, String)>),
    Color,
    Toggle,
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    /// VIA value name, e.g. `id_qmk_rgb_matrix_effect`.
    pub name: String,
    /// Visibility expression such as `{id_x} > 1 && {id_x} < 23`.
    pub show_if: Option<String>,
    pub section: String,
    pub label: String,
    pub kind: ItemKind,
    pub channel: u8,
    pub value_id: u8,
}

impl MenuItem {
    pub fn id(&self) -> String {
        format!("via.{}.{}", self.channel, self.value_id)
    }

    /// Bytes the value occupies on the wire.
    pub fn width(&self) -> usize {
        match self.kind {
            ItemKind::Range { max, .. } if max > 255 => 2,
            ItemKind::Color => 2,
            _ => 1,
        }
    }
}

pub fn parse(menus: &[Json]) -> Vec<MenuItem> {
    let mut out = Vec::new();
    for m in menus {
        walk(m, "", &mut out);
    }
    out
}

fn walk(node: &Json, section: &str, out: &mut Vec<MenuItem>) {
    let label = node.get("label").and_then(Json::as_str).unwrap_or(section);
    let Some(content) = node.get("content").and_then(Json::as_array) else { return };
    if let Some(Json::String(_)) = content.first() {
        if let Some(item) = item(node, section, label, content) {
            out.push(item);
        }
        return;
    }
    for child in content {
        walk(child, label, out);
    }
}

fn item(node: &Json, section: &str, label: &str, content: &[Json]) -> Option<MenuItem> {
    let name = content.first()?.as_str()?.to_string();
    let show_if = node.get("showIf").and_then(Json::as_str).map(str::to_string);
    let channel = content.get(1)?.as_u64()? as u8;
    let value_id = content.get(2)?.as_u64()? as u8;
    let options = node.get("options").and_then(Json::as_array);
    let kind = match node.get("type").and_then(Json::as_str)? {
        "range" => {
            let o = options.map(|o| o.iter().filter_map(Json::as_i64).collect::<Vec<_>>());
            let (min, max) = match o.as_deref() {
                Some([a, b]) => (*a, *b),
                _ => (0, 255),
            };
            ItemKind::Range { min, max }
        }
        "dropdown" => ItemKind::Dropdown(
            options?
                .iter()
                .enumerate()
                .filter_map(|(i, o)| match o {
                    Json::Array(pair) => Some((pair.get(1)?.as_i64()?, pair.first()?.as_str()?.to_string())),
                    Json::String(s) => Some((i as i64, s.clone())),
                    _ => None,
                })
                .collect(),
        ),
        "color" => ItemKind::Color,
        "toggle" => ItemKind::Toggle,
        _ => return None,
    };
    Some(MenuItem { name, show_if, section: section.to_string(), label: label.to_string(), kind, channel, value_id })
}

/// VIA colours are hue + saturation (full value), each 0-255.
pub fn hs_to_rgb(h: u8, s: u8) -> [u8; 3] {
    let h = h as f32 / 255.0 * 6.0;
    let s = s as f32 / 255.0;
    let i = h.floor() as i32 % 6;
    let f = h - h.floor();
    let (p, q, t) = (1.0 - s, 1.0 - s * f, 1.0 - s * (1.0 - f));
    let (r, g, b) = match i {
        0 => (1.0, t, p),
        1 => (q, 1.0, p),
        2 => (p, 1.0, t),
        3 => (p, q, 1.0),
        4 => (t, p, 1.0),
        _ => (1.0, p, q),
    };
    [(r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8]
}

pub fn rgb_to_hs([r, g, b]: [u8; 3]) -> (u8, u8) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let s = if max == 0.0 { 0.0 } else { d / max };
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    ((h / 6.0 * 255.0).round() as u8, (s * 255.0).round() as u8)
}

pub fn hsv_to_rgb([h, s, v]: [u8; 3]) -> [u8; 3] {
    let [r, g, b] = hs_to_rgb(h, s);
    let scale = |c: u8| (c as u16 * v as u16 / 255) as u8;
    [scale(r), scale(g), scale(b)]
}

pub fn rgb_to_hsv(rgb: [u8; 3]) -> [u8; 3] {
    let (h, s) = rgb_to_hs(rgb);
    [h, s, rgb.into_iter().max().unwrap_or(0)]
}

/// Evaluates a VIA `showIf` expression. `value` resolves `{id_name}`.
/// Supports numbers, ==, !=, <, <=, >, >=, &&, ||, ! and parentheses.
/// Unparseable expressions evaluate to true (show the control).
pub fn show_if(expr: &str, value: &dyn Fn(&str) -> Option<i64>) -> bool {
    let tokens = match tokenize(expr, value) {
        Some(t) => t,
        None => return true,
    };
    let mut p = Parser { t: &tokens, i: 0 };
    match p.or() {
        Some(v) if p.i == tokens.len() => v != 0,
        _ => true,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(i64),
    Op(&'static str),
}

fn tokenize(s: &str, value: &dyn Fn(&str) -> Option<i64>) -> Option<Vec<Tok>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'{' {
            let end = s[i..].find('}')? + i;
            out.push(Tok::Num(value(s[i + 1..end].trim())?));
            i = end + 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Tok::Num(s[start..i].parse().ok()?));
        } else {
            let two = s.get(i..i + 2).unwrap_or("");
            let op = ["&&", "||", "==", "!=", "<=", ">="].into_iter().find(|o| *o == two);
            if let Some(op) = op {
                out.push(Tok::Op(op));
                i += 2;
            } else {
                out.push(Tok::Op(match c {
                    b'<' => "<",
                    b'>' => ">",
                    b'!' => "!",
                    b'(' => "(",
                    b')' => ")",
                    _ => return None,
                }));
                i += 1;
            }
        }
    }
    Some(out)
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
}

impl Parser<'_> {
    fn eat(&mut self, op: &str) -> bool {
        if self.t.get(self.i) == Some(&Tok::Op(match op {
            "&&" => "&&",
            "||" => "||",
            "(" => "(",
            ")" => ")",
            "!" => "!",
            _ => return false,
        })) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Option<i64> {
        let mut v = self.and()?;
        while self.eat("||") {
            let r = self.and()?;
            v = (v != 0 || r != 0) as i64;
        }
        Some(v)
    }

    fn and(&mut self) -> Option<i64> {
        let mut v = self.cmp()?;
        while self.eat("&&") {
            let r = self.cmp()?;
            v = (v != 0 && r != 0) as i64;
        }
        Some(v)
    }

    fn cmp(&mut self) -> Option<i64> {
        let a = self.unary()?;
        if let Some(Tok::Op(op)) = self.t.get(self.i).cloned() {
            if ["==", "!=", "<", "<=", ">", ">="].contains(&op) {
                self.i += 1;
                let b = self.unary()?;
                return Some(match op {
                    "==" => a == b,
                    "!=" => a != b,
                    "<" => a < b,
                    "<=" => a <= b,
                    ">" => a > b,
                    _ => a >= b,
                } as i64);
            }
        }
        Some(a)
    }

    fn unary(&mut self) -> Option<i64> {
        if self.eat("!") {
            return Some((self.unary()? == 0) as i64);
        }
        if self.eat("(") {
            let v = self.or()?;
            return self.eat(")").then_some(v);
        }
        match self.t.get(self.i)? {
            Tok::Num(n) => {
                self.i += 1;
                Some(*n)
            }
            Tok::Op(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_if_expressions_from_k2_he() {
        let eff = |e: i64| move |n: &str| (n == "id_qmk_rgb_matrix_effect").then_some(e);
        let speed = "{id_qmk_rgb_matrix_effect} > 1 && {id_qmk_rgb_matrix_effect} < 23";
        let color = "({id_qmk_rgb_matrix_effect} >= 1 && {id_qmk_rgb_matrix_effect} <= 3) || ({id_qmk_rgb_matrix_effect} >= 18 && {id_qmk_rgb_matrix_effect} <= 20) || {id_qmk_rgb_matrix_effect} == 22";
        assert!(show_if(speed, &eff(11)));
        assert!(!show_if(speed, &eff(1)));
        assert!(show_if(color, &eff(2)));
        assert!(!show_if(color, &eff(11)));
        assert!(show_if(color, &eff(22)));
        assert!(show_if("garbage $$", &eff(0)));
    }

    #[test]
    fn colour_roundtrip() {
        for (h, s) in [(0, 255), (85, 255), (170, 128), (248, 255)] {
            let (h2, s2) = rgb_to_hs(hs_to_rgb(h, s));
            assert!((h as i32 - h2 as i32).abs() <= 1 && (s as i32 - s2 as i32).abs() <= 1, "{h},{s}");
        }
    }
}
