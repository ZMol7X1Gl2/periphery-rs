//! Brand-neutral device model. Drivers describe their settings as data; the
//! GUI and CLI render whatever a device reports without knowing the brand.
//!
//! Device I/O is async so the same drivers run on the desktop (over hidapi)
//! and in the browser (over WebHID); see [`hid`]. Implement the traits with
//! `#[async_trait(?Send)]`.

use anyhow::{Result, bail};
pub mod backup;
pub mod unknown;

pub use async_trait::async_trait;
pub use hid_transport as hid;
use hid::{Api, Filter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Other,
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub brand: String,
    pub model: String,
    pub kind: DeviceKind,
    pub vid: u16,
    pub pid: u16,
}

impl DeviceInfo {
    pub fn title(&self) -> String {
        if self.model.to_lowercase().starts_with(&self.brand.to_lowercase()) {
            self.model.clone()
        } else {
            format!("{} {}", self.brand, self.model)
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Color([u8; 3]),
    Text(String),
    /// Triggers an [`Kind::Action`].
    Unit,
}

impl Value {
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Value::Int(v) => Some(v),
            Value::Bool(b) => Some(b as i64),
            _ => None,
        }
    }

    pub fn int(&self) -> Result<i64> {
        match self.as_int() {
            Some(v) => Ok(v),
            None => bail!("expected a number, got {self:?}"),
        }
    }

    pub fn bool(&self) -> Result<bool> {
        Ok(self.int()? != 0)
    }

    pub fn color(&self) -> Result<[u8; 3]> {
        match self {
            Value::Color(c) => Ok(*c),
            _ => bail!("expected a colour, got {self:?}"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Kind {
    Toggle,
    /// Integer range. The UI shows `value * scale` followed by `unit`.
    Range {
        min: i64,
        max: i64,
        step: i64,
        scale: f64,
        unit: String,
    },
    Choice(Vec<(i64, String)>),
    Color,
    Text {
        max_len: usize,
    },
    /// A button; `confirm` asks before running it.
    Action {
        confirm: Option<String>,
    },
    /// Read-only text.
    Info,
}

impl Kind {
    pub fn range(min: i64, max: i64) -> Kind {
        Kind::Range { min, max, step: 1, scale: 1.0, unit: String::new() }
    }

    pub fn range_unit(min: i64, max: i64, step: i64, scale: f64, unit: &str) -> Kind {
        Kind::Range { min, max, step, scale, unit: unit.into() }
    }

    pub fn choice<S: Into<String>>(items: impl IntoIterator<Item = (i64, S)>) -> Kind {
        Kind::Choice(items.into_iter().map(|(v, s)| (v, s.into())).collect())
    }
}

/// What a setting means, so the UI can lay out every brand's mouse the same
/// way. Settings without a role are device-specific and shown in their own
/// section under their group's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Onboard profile selector.
    Profile,
    /// Where the settings come from (onboard memory or software).
    SettingsSource,
    /// Polling rate in Hz (a [`Kind::Choice`] whose values are Hz).
    PollingRate,
    /// Number of enabled DPI stages ([`Kind::Range`]).
    StageCount,
    /// Selected DPI stage ([`Kind::Choice`] of stage indices).
    ActiveStage,
    /// DPI of stage n.
    Dpi(u8),
    /// Indicator colour of DPI stage n.
    StageColor(u8),
    /// Binding of button n.
    Button(u8),
    /// Extra option for button n's binding (macro text, fire rate…).
    ButtonOption(u8),
    LiftOff,
    Debounce,
    MotionSync,
    AngleSnap,
    Ripple,
    Lighting,
    Sleep,
    Wireless,
    FactoryReset,
}

#[derive(Clone, Debug)]
pub struct Setting {
    pub id: String,
    pub label: String,
    pub help: Option<String>,
    pub kind: Kind,
    pub value: Value,
    pub role: Option<Role>,
}

impl Setting {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: Kind, value: Value) -> Self {
        Setting { id: id.into(), label: label.into(), help: None, kind, value, role: None }
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    pub fn toggle(id: impl Into<String>, label: impl Into<String>, on: bool) -> Self {
        Self::new(id, label, Kind::Toggle, Value::Bool(on))
    }

    pub fn action(id: impl Into<String>, label: impl Into<String>, confirm: Option<&str>) -> Self {
        Self::new(id, label, Kind::Action { confirm: confirm.map(Into::into) }, Value::Unit)
    }

    pub fn help(mut self, text: impl Into<String>) -> Self {
        self.help = Some(text.into());
        self
    }

    /// Validates a value against this setting's kind.
    pub fn check(&self, v: &Value) -> Result<()> {
        match (&self.kind, v) {
            (Kind::Toggle, Value::Bool(_)) => Ok(()),
            (Kind::Range { min, max, .. }, Value::Int(x)) if (min..=max).contains(&x) => Ok(()),
            (Kind::Range { min, max, .. }, _) => bail!("{} must be {min}..={max}", self.label),
            (Kind::Choice(opts), Value::Int(x)) if opts.iter().any(|(o, _)| o == x) => Ok(()),
            (Kind::Choice(_), _) => bail!("invalid choice for {}", self.label),
            (Kind::Color, Value::Color(_)) => Ok(()),
            (Kind::Text { max_len }, Value::Text(t)) if t.chars().count() <= *max_len => Ok(()),
            (Kind::Action { .. }, Value::Unit) => Ok(()),
            (Kind::Info, _) => bail!("{} is read-only", self.label),
            _ => bail!("wrong value type for {}", self.label),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    pub label: String,
    pub settings: Vec<Setting>,
}

impl Group {
    pub fn new(label: impl Into<String>) -> Self {
        Group { label: label.into(), settings: Vec::new() }
    }

    pub fn push(&mut self, s: Setting) {
        self.settings.push(s);
    }

    /// Pushes `s` tagged with `role`.
    pub fn push_as(&mut self, role: Role, s: Setting) {
        self.settings.push(s.role(role));
    }
}

/// One key cap as drawn on screen, in key units (1.0 = one standard key).
#[derive(Clone, Debug)]
pub struct LayoutKey {
    pub row: u8,
    pub col: u8,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug)]
pub struct KeycodeGroup {
    pub label: String,
    pub codes: Vec<(u16, String)>,
}

/// A setting with one value per key (per-key actuation, per-key colour…).
/// The UI edits it by selecting keys on the drawn keyboard.
#[derive(Clone, Debug)]
pub struct KeyProperty {
    pub id: String,
    pub label: String,
    pub help: Option<String>,
    pub kind: Kind,
    /// `values[row * cols + col]`; `None` = follows the default.
    pub values: Vec<Option<Value>>,
    /// Shown for keys whose value is `None`, e.g. "global".
    pub default_label: String,
    /// Whether keys can be reset to the default.
    pub resettable: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeyPair {
    pub a: (u8, u8),
    pub b: (u8, u8),
    pub mode: i64,
}

/// A list of key pairs with a mode, e.g. snap-tap / SOCD.
#[derive(Clone, Debug)]
pub struct KeyPairList {
    pub id: String,
    pub label: String,
    pub help: Option<String>,
    pub max: usize,
    pub modes: Vec<(i64, String)>,
    pub pairs: Vec<KeyPair>,
}

#[derive(Clone, Debug)]
pub struct Keymap {
    pub rows: u8,
    pub cols: u8,
    pub layout: Vec<LayoutKey>,
    pub layer_names: Vec<String>,
    /// `codes[layer][row * cols + col]`
    pub codes: Vec<Vec<u16>>,
    /// Display labels parallel to `codes`.
    pub labels: Vec<Vec<String>>,
    /// Keycodes offered in the picker.
    pub catalog: Vec<KeycodeGroup>,
    pub properties: Vec<KeyProperty>,
    pub pair_lists: Vec<KeyPairList>,
}

impl Keymap {
    pub fn label_for(&self, code: u16) -> Option<&str> {
        self.catalog
            .iter()
            .flat_map(|g| g.codes.iter())
            .find(|(c, _)| *c == code)
            .map(|(_, l)| l.as_str())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Read-only facts shown in the header (battery, firmware, connection…).
    pub status: Vec<(String, String)>,
    pub groups: Vec<Group>,
    pub keymap: Option<Keymap>,
    /// Shown instead of settings when the device can't be configured right now.
    pub notice: Option<String>,
}

impl Snapshot {
    pub fn find(&self, id: &str) -> Option<&Setting> {
        self.groups.iter().flat_map(|g| g.settings.iter()).find(|s| s.id == id)
    }

    /// Whether the driver tagged any setting with a [`Role`].
    pub fn has_roles(&self) -> bool {
        self.groups.iter().flat_map(|g| &g.settings).any(|s| s.role.is_some())
    }
}

#[async_trait(?Send)]
pub trait Device {
    fn info(&self) -> &DeviceInfo;

    /// Reads the complete current state from the device.
    async fn snapshot(&mut self) -> Result<Snapshot>;

    /// Applies one setting by id.
    async fn set(&mut self, id: &str, value: &Value) -> Result<()>;

    async fn set_key(&mut self, _layer: u8, _row: u8, _col: u8, _code: u16) -> Result<()> {
        bail!("this device has no keymap")
    }

    /// Sets a [`KeyProperty`] for several keys; `None` resets them to default.
    async fn set_key_property(&mut self, _id: &str, _keys: &[(u8, u8)], _value: Option<&Value>) -> Result<()> {
        bail!("this device has no per-key settings")
    }

    /// Replaces the whole [`KeyPairList`] `id`.
    async fn set_key_pairs(&mut self, _id: &str, _pairs: &[KeyPair]) -> Result<()> {
        bail!("this device has no key pair settings")
    }

    /// Returns at once: true when the device reports changes made on the
    /// device itself (on-board buttons), so the UI should refresh.
    async fn poll_changes(&mut self) -> Result<bool> {
        Ok(false)
    }
}

#[async_trait(?Send)]
pub trait Driver: Sync {
    fn name(&self) -> &'static str;

    /// The HID interfaces this driver talks to. The browser only shows a page
    /// devices the user picked, and the picker lists only matching ones.
    fn filters(&self) -> Vec<Filter>;

    /// Whether this driver handles the interface, even when nothing could be
    /// opened right now (e.g. a receiver whose mouse sleeps). Devices no
    /// driver claims are offered for a support request. Defaults to
    /// [`Driver::filters`].
    fn claims(&self, info: &hid::DeviceInfo) -> bool {
        self.filters().iter().any(|f| f.matches(info))
    }

    /// Opens every supported device in the current HID enumeration.
    async fn discover(&self, api: &Api) -> Vec<Result<Box<dyn Device>>>;
}

/// Every driver's filters, for [`Api::request`].
pub fn filters(drivers: &[&dyn Driver]) -> Vec<Filter> {
    drivers.iter().flat_map(|d| d.filters()).collect()
}

/// Runs every driver; returns opened devices and per-device errors.
pub async fn discover_all(api: &Api, drivers: &[&dyn Driver]) -> (Vec<Box<dyn Device>>, Vec<String>) {
    let mut devices = Vec::new();
    let mut errors = Vec::new();
    for d in drivers {
        for r in d.discover(api).await {
            match r {
                Ok(dev) => devices.push(dev),
                Err(e) => errors.push(format!("{}: {e:#}", d.name())),
            }
        }
    }
    (devices, errors)
}
