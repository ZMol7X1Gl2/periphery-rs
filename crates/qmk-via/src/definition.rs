//! Keyboard definitions (layout, matrix, lighting menus). Keychron serves
//! pre-parsed VIA v3 definitions per product; other boards can use a file
//! dropped into the definitions folder with the same shape.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Matrix {
    pub rows: u8,
    pub cols: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyPos {
    pub row: u8,
    pub col: u8,
    pub x: f32,
    pub y: f32,
    #[serde(default = "one")]
    pub w: f32,
    #[serde(default = "one")]
    pub h: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layouts {
    pub keys: Vec<KeyPos>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomKeycode {
    pub name: String,
    #[serde(rename = "shortName")]
    pub short_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Definition {
    pub name: String,
    pub matrix: Matrix,
    #[serde(default)]
    pub layers: Vec<String>,
    #[serde(default)]
    pub feature: Vec<String>,
    #[serde(default)]
    pub menus: Vec<serde_json::Value>,
    #[serde(default, rename = "customKeycodes")]
    pub custom_keycodes: Vec<CustomKeycode>,
    pub layouts: Layouts,
}

impl Definition {
    pub fn custom_labels(&self) -> Vec<String> {
        self.custom_keycodes
            .iter()
            .map(|c| c.short_name.clone().unwrap_or_else(|| c.name.clone()))
            .collect()
    }

    pub fn has_feature(&self, f: &str) -> bool {
        self.feature.iter().any(|x| x == f)
    }
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("periphery").join("definitions")
}

fn cache_path(vid: u16, pid: u16) -> PathBuf {
    cache_dir().join(format!("{vid:04x}_{pid:04x}.json"))
}

pub const KEYCHRON_VID: u16 = 0x3434;

/// Keychron launcher URL: `static/device/{vid<<16|pid}/json/{v2|v3}[-{ver}].json`.
fn keychron_url(vid: u16, pid: u16, via_protocol: u16, json_version: u32) -> String {
    let vpid = (vid as u32) << 16 | pid as u32;
    let base = if via_protocol >= 11 { "v3" } else { "v2" };
    let file = if json_version > 0 { format!("{base}-{json_version}.json") } else { format!("{base}.json") };
    format!("https://launcher.keychron.com/static/device/{vpid}/json/{file}")
}

/// Loads a cached definition, downloading Keychron's on first use.
pub fn load(vid: u16, pid: u16, via_protocol: u16, json_version: u32) -> Result<Option<Definition>> {
    let path = cache_path(vid, pid);
    let text = if path.exists() {
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
    } else if vid == KEYCHRON_VID {
        let url = keychron_url(vid, pid, via_protocol, json_version);
        let resp = ureq::get(&url).timeout(Duration::from_secs(10)).call();
        let text = match resp {
            Ok(r) => r.into_string().context("reading definition")?,
            Err(ureq::Error::Status(404, _)) => return Ok(None),
            Err(e) => bail!("downloading keyboard definition: {e}"),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(&path, &text).ok();
        text
    } else {
        return Ok(None);
    };
    let def: Definition = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a supported definition (needs pre-parsed layouts.keys)", path.display()))?;
    Ok(Some(def))
}
