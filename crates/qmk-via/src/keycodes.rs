//! QMK keycodes as used by VIA protocol 12+ ("v3" numbering).

/// A named group of keycodes for a picker.
#[derive(Clone, Debug)]
pub struct KeycodeGroup {
    pub label: String,
    pub codes: Vec<(u16, String)>,
}

const BASIC: &[(u16, &str)] = &[
    (0x00, ""),
    (0x01, "trns"),
    (0x28, "Enter"),
    (0x29, "Esc"),
    (0x2A, "Bksp"),
    (0x2B, "Tab"),
    (0x2C, "Space"),
    (0x2D, "-"),
    (0x2E, "="),
    (0x2F, "["),
    (0x30, "]"),
    (0x31, "\\"),
    (0x32, "#~"),
    (0x33, ";"),
    (0x34, "'"),
    (0x35, "`"),
    (0x36, ","),
    (0x37, "."),
    (0x38, "/"),
    (0x39, "Caps"),
    (0x46, "PrtSc"),
    (0x47, "ScrLk"),
    (0x48, "Pause"),
    (0x49, "Ins"),
    (0x4A, "Home"),
    (0x4B, "PgUp"),
    (0x4C, "Del"),
    (0x4D, "End"),
    (0x4E, "PgDn"),
    (0x4F, "Right"),
    (0x50, "Left"),
    (0x51, "Down"),
    (0x52, "Up"),
    (0x64, "ISO \\"),
    (0x65, "Menu"),
];

const NUMPAD: &[(u16, &str)] = &[
    (0x53, "NumLk"),
    (0x54, "N/"),
    (0x55, "N*"),
    (0x56, "N-"),
    (0x57, "N+"),
    (0x58, "NEnt"),
    (0x59, "N1"),
    (0x5A, "N2"),
    (0x5B, "N3"),
    (0x5C, "N4"),
    (0x5D, "N5"),
    (0x5E, "N6"),
    (0x5F, "N7"),
    (0x60, "N8"),
    (0x61, "N9"),
    (0x62, "N0"),
    (0x63, "N."),
    (0x67, "N="),
];

const MODIFIERS: &[(u16, &str)] = &[
    (0xE0, "LCtrl"),
    (0xE1, "LShift"),
    (0xE2, "LAlt"),
    (0xE3, "LGui"),
    (0xE4, "RCtrl"),
    (0xE5, "RShift"),
    (0xE6, "RAlt"),
    (0xE7, "RGui"),
];

const MEDIA: &[(u16, &str)] = &[
    (0xA5, "Power"),
    (0xA6, "Sleep"),
    (0xA7, "Wake"),
    (0xA8, "Mute"),
    (0xA9, "Vol+"),
    (0xAA, "Vol-"),
    (0xAB, "Next"),
    (0xAC, "Prev"),
    (0xAD, "Stop"),
    (0xAE, "Play"),
    (0xAF, "Media"),
    (0xB0, "Eject"),
    (0xB1, "Mail"),
    (0xB2, "Calc"),
    (0xB3, "MyPC"),
    (0xB4, "Search"),
    (0xB5, "WebHome"),
    (0xB6, "WebBack"),
    (0xB7, "WebFwd"),
    (0xB8, "WebStop"),
    (0xB9, "WebRefr"),
    (0xBA, "WebFav"),
    (0xBB, "FFwd"),
    (0xBC, "Rewind"),
    (0xBD, "Bri+"),
    (0xBE, "Bri-"),
    (0xBF, "CtlPanel"),
    (0xC0, "Assist"),
    (0xC1, "MCtl"),
    (0xC2, "LPad"),
];

const MOUSE: &[(u16, &str)] = &[
    (0xCD, "Ms Up"),
    (0xCE, "Ms Down"),
    (0xCF, "Ms Left"),
    (0xD0, "Ms Right"),
    (0xD1, "Btn1"),
    (0xD2, "Btn2"),
    (0xD3, "Btn3"),
    (0xD4, "Btn4"),
    (0xD5, "Btn5"),
    (0xD9, "Wh Up"),
    (0xDA, "Wh Down"),
    (0xDB, "Wh Left"),
    (0xDC, "Wh Right"),
];

const LIGHTING: &[(u16, &str)] = &[
    (0x7013, "NKRO Tog"),
    (0x7820, "RGB Tog"),
    (0x7821, "RGB Mode+"),
    (0x7822, "RGB Mode-"),
    (0x7823, "Hue+"),
    (0x7824, "Hue-"),
    (0x7825, "Sat+"),
    (0x7826, "Sat-"),
    (0x7827, "Bright+"),
    (0x7828, "Bright-"),
    (0x7829, "Speed+"),
    (0x782A, "Speed-"),
];

const QK_MODS: u16 = 0x0100;
const QK_MOD_TAP: u16 = 0x2000;
const QK_LAYER_TAP: u16 = 0x4000;
const QK_LAYER_MOD: u16 = 0x5000;
const QK_TO: u16 = 0x5200;
const QK_MOMENTARY: u16 = 0x5220;
const QK_DEF_LAYER: u16 = 0x5240;
const QK_TOGGLE_LAYER: u16 = 0x5260;
const QK_ONE_SHOT_LAYER: u16 = 0x5280;
const QK_ONE_SHOT_MOD: u16 = 0x52A0;
const QK_LAYER_TAP_TOGGLE: u16 = 0x52C0;
const QK_MACRO: u16 = 0x7700;
const QK_KB: u16 = 0x7E00;

fn basic_label(code: u16) -> Option<String> {
    match code {
        0x04..=0x1D => Some(((b'A' + (code - 0x04) as u8) as char).to_string()),
        0x1E..=0x26 => Some(((b'1' + (code - 0x1E) as u8) as char).to_string()),
        0x27 => Some("0".into()),
        0x3A..=0x45 => Some(format!("F{}", code - 0x3A + 1)),
        0x68..=0x73 => Some(format!("F{}", code - 0x68 + 13)),
        _ => [BASIC, NUMPAD, MODIFIERS, MEDIA, MOUSE]
            .iter()
            .flat_map(|t| t.iter())
            .find(|(c, _)| *c == code)
            .map(|(_, l)| l.to_string()),
    }
}

/// 5-bit QMK mod mask (bit 4 = right-hand) as a short prefix like "C+S+".
fn mods_label(mods: u16) -> String {
    let right = mods & 0x10 != 0;
    let mut s = String::new();
    for (bit, name) in [(1, "C"), (2, "S"), (4, "A"), (8, "G")] {
        if mods & bit != 0 {
            if right {
                s.push('R');
            }
            s.push_str(name);
            s.push('+');
        }
    }
    s
}

pub fn label(code: u16, custom: &[String]) -> String {
    let basic = |c: u16| basic_label(c).unwrap_or_else(|| format!("{c:#04x}"));
    match code {
        0x0000..=0x00FF => basic(code),
        QK_MODS..=0x1FFF => format!("{}{}", mods_label(code >> 8), basic(code & 0xFF)),
        QK_MOD_TAP..=0x3FFF => format!("MT({}{})", mods_label((code >> 8) & 0x1F), basic(code & 0xFF)),
        QK_LAYER_TAP..=0x4FFF => format!("LT{}({})", (code >> 8) & 0xF, basic(code & 0xFF)),
        QK_LAYER_MOD..=0x51FF => format!("LM{}({})", (code >> 5) & 0xF, mods_label(code & 0x1F)),
        QK_TO..=0x521F => format!("TO({})", code & 0x1F),
        QK_MOMENTARY..=0x523F => format!("MO({})", code & 0x1F),
        QK_DEF_LAYER..=0x525F => format!("DF({})", code & 0x1F),
        QK_TOGGLE_LAYER..=0x527F => format!("TG({})", code & 0x1F),
        QK_ONE_SHOT_LAYER..=0x529F => format!("OSL({})", code & 0x1F),
        QK_ONE_SHOT_MOD..=0x52BF => format!("OSM({})", mods_label(code & 0x1F)),
        QK_LAYER_TAP_TOGGLE..=0x52DF => format!("TT({})", code & 0x1F),
        QK_MACRO..=0x777F => format!("M{}", code - QK_MACRO),
        QK_KB..=0x7E3F => custom
            .get((code - QK_KB) as usize)
            .cloned()
            .unwrap_or_else(|| format!("KB{}", code - QK_KB)),
        _ => LIGHTING
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, l)| l.to_string())
            .unwrap_or_else(|| format!("{code:#06x}")),
    }
}

pub fn catalog(layers: u8, macros: u8, custom: &[String]) -> Vec<KeycodeGroup> {
    let group = |label: &str, codes: Vec<(u16, String)>| KeycodeGroup { label: label.into(), codes };
    let table = |t: &[(u16, &str)]| t.iter().map(|(c, l)| (*c, l.to_string())).collect::<Vec<_>>();

    let mut basic: Vec<(u16, String)> = (0x04..=0x27).map(|c| (c, label(c, &[]))).collect();
    basic.extend((0x3A..=0x45).map(|c| (c, label(c, &[]))));
    basic.extend(table(&BASIC[2..]));
    let mut special = vec![(0x00, "None".to_string()), (0x01, "Transparent".to_string())];
    special.extend((0x68..=0x73).map(|c| (c, label(c, &[]))));

    let mut layer_codes = Vec::new();
    for (base, name) in [
        (QK_MOMENTARY, "MO"),
        (QK_TOGGLE_LAYER, "TG"),
        (QK_TO, "TO"),
        (QK_DEF_LAYER, "DF"),
        (QK_ONE_SHOT_LAYER, "OSL"),
        (QK_LAYER_TAP_TOGGLE, "TT"),
    ] {
        layer_codes.extend((0..layers as u16).map(|l| (base + l, format!("{name}({l})"))));
    }

    let mut groups = vec![
        group("Basic", basic),
        group("Modifiers", table(MODIFIERS)),
        group("Media & system", table(MEDIA)),
        group("Numpad", table(NUMPAD)),
        group("Mouse", table(MOUSE)),
        group("Layers", layer_codes),
        group("Lighting", table(LIGHTING)),
    ];
    if macros > 0 {
        groups.push(group("Macros", (0..macros as u16).map(|m| (QK_MACRO + m, format!("M{m}"))).collect()));
    }
    if !custom.is_empty() {
        groups.push(group(
            "Keyboard",
            custom.iter().enumerate().map(|(i, n)| (QK_KB + i as u16, n.clone())).collect(),
        ));
    }
    groups.push(group("Special", special));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_k2_he_keymap() {
        // First row of layer 0 as read from a Keychron K2 HE.
        let custom: Vec<String> = ["L Opt", "R Opt", "L Cmd", "R Cmd", "MCtl", "LPad", "Task", "File", "SShot"]
            .map(String::from)
            .to_vec();
        let row = [0x0029, 0x00be, 0x00bd, 0x7e04, 0x7e05, 0x7828, 0x7827, 0x00ac, 0x00ae, 0x00ab, 0x00a8, 0x00aa, 0x00a9, 0x7e08];
        let labels: Vec<_> = row.iter().map(|c| label(*c, &custom)).collect();
        assert_eq!(
            labels,
            ["Esc", "Bri-", "Bri+", "MCtl", "LPad", "Bright-", "Bright+", "Prev", "Play", "Next", "Mute", "Vol-", "Vol+", "SShot"]
        );
        assert_eq!(label(0x0204, &[]), "S+A");
        assert_eq!(label(0x5221, &[]), "MO(1)");
        assert_eq!(label(0x4104, &[]), "LT1(A)");
    }
}
