//! Keyboard definitions (layout, matrix, lighting menus). Keychron serves
//! pre-parsed VIA v3 definitions per product; other boards can use a file
//! dropped into the definitions folder with the same shape.

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

pub const KEYCHRON_VID: u16 = 0x3434;

/// Keychron launcher URL: `static/device/{vid<<16|pid}/json/{v2|v3}[-{ver}].json`.
fn keychron_url(vid: u16, pid: u16, via_protocol: u16, json_version: u32) -> String {
    let vpid = (vid as u32) << 16 | pid as u32;
    let base = if via_protocol >= 11 { "v3" } else { "v2" };
    let file = if json_version > 0 { format!("{base}-{json_version}.json") } else { format!("{base}.json") };
    format!("https://launcher.keychron.com/static/device/{vpid}/json/{file}")
}

fn cache_name(vid: u16, pid: u16) -> String {
    format!("{vid:04x}_{pid:04x}.json")
}

/// Loads a cached definition, downloading Keychron's on first use.
pub async fn load(vid: u16, pid: u16, via_protocol: u16, json_version: u32) -> Result<Option<Definition>> {
    let name = cache_name(vid, pid);
    let text = match cache::read(&name)? {
        Some(text) => text,
        None if vid == KEYCHRON_VID => {
            let Some(text) = download(&keychron_url(vid, pid, via_protocol, json_version)).await? else {
                return Ok(None);
            };
            cache::write(&name, &text);
            text
        }
        None => return Ok(None),
    };
    let def: Definition = serde_json::from_str(&text)
        .with_context(|| format!("{name} is not a supported definition (needs pre-parsed layouts.keys)"))?;
    Ok(Some(def))
}

/// Where definitions are kept, for messages asking the user to add one.
pub fn cache_location() -> String {
    cache::location()
}

#[cfg(not(target_arch = "wasm32"))]
async fn download(url: &str) -> Result<Option<String>> {
    match ureq::get(url).timeout(std::time::Duration::from_secs(10)).call() {
        Ok(r) => Ok(Some(r.into_string().context("reading definition")?)),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => bail!("downloading keyboard definition: {e}"),
    }
}

#[cfg(target_arch = "wasm32")]
async fn download(url: &str) -> Result<Option<String>> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let err = |e: wasm_bindgen::JsValue| anyhow::anyhow!("downloading keyboard definition: {e:?}");
    let window = web_sys::window().context("no browser window")?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(url)).await.map_err(err)?.unchecked_into();
    if resp.status() == 404 {
        return Ok(None);
    }
    if !resp.ok() {
        bail!("downloading keyboard definition: HTTP {}", resp.status());
    }
    let text = JsFuture::from(resp.text().map_err(err)?).await.map_err(err)?;
    Ok(text.as_string())
}

#[cfg(not(target_arch = "wasm32"))]
mod cache {
    use std::path::PathBuf;

    use anyhow::{Context, Result};

    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("periphery").join("definitions")
    }

    pub fn location() -> String {
        dir().display().to_string()
    }

    pub fn read(name: &str) -> Result<Option<String>> {
        let path = dir().join(name);
        if !path.exists() {
            return Ok(None);
        }
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display())).map(Some)
    }

    pub fn write(name: &str, text: &str) {
        let dir = dir();
        std::fs::create_dir_all(&dir).ok();
        std::fs::write(dir.join(name), text).ok();
    }
}

/// The browser keeps definitions in local storage.
#[cfg(target_arch = "wasm32")]
mod cache {
    use anyhow::Result;

    const PREFIX: &str = "periphery.definition.";

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    pub fn location() -> String {
        "the browser's storage for this site".into()
    }

    pub fn read(name: &str) -> Result<Option<String>> {
        Ok(storage().and_then(|s| s.get_item(&format!("{PREFIX}{name}")).ok().flatten()))
    }

    pub fn write(name: &str, text: &str) {
        if let Some(s) = storage() {
            let _ = s.set_item(&format!("{PREFIX}{name}"), text);
        }
    }
}
