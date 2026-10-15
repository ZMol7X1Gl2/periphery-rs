//! HID devices no driver handles, so users can ask for them to be supported.

use std::collections::BTreeMap;

use crate::Driver;
use crate::hid::Api;

/// One physical device (all its collections) that no driver claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownDevice {
    pub vid: u16,
    pub pid: u16,
    /// Empty when the device reports none.
    pub product: String,
    /// Empty when unknown (always in the browser).
    pub manufacturer: String,
    /// (usage page, usage) of every top-level collection.
    pub collections: Vec<(u16, u16)>,
    /// USB interface numbers; empty when unknown (always in the browser).
    pub interfaces: Vec<i32>,
}

impl UnknownDevice {
    /// `vid:pid` in lowercase hex, e.g. `046d:c547`.
    pub fn id(&self) -> String {
        format!("{:04x}:{:04x}", self.vid, self.pid)
    }

    pub fn name(&self) -> String {
        match (self.manufacturer.trim(), self.product.trim()) {
            (_, "") => format!("Unknown device {}", self.id()),
            ("", p) => p.to_string(),
            (m, p) if p.to_lowercase().starts_with(&m.to_lowercase()) => p.to_string(),
            (m, p) => format!("{m} {p}"),
        }
    }

    /// A mouse or keyboard with a vendor-defined interface: what configurable
    /// peripherals look like. Plain office mice and keyboards have no vendor
    /// interface.
    pub fn is_configurable(&self) -> bool {
        let input = self.collections.iter().any(|&(page, usage)| page == 0x01 && matches!(usage, 0x02 | 0x06));
        let vendor = self.collections.iter().any(|&(page, _)| page >= 0xFF00);
        input && vendor
    }

    /// Plain-text facts for a support request; no serial numbers or paths.
    pub fn report(&self) -> String {
        let collections: Vec<String> = self.collections.iter().map(|(p, u)| format!("{p:04x}:{u:04x}")).collect();
        let mut out = format!("VID:PID: {}\n", self.id());
        if !self.product.is_empty() {
            out += &format!("Product: {}\n", self.product);
        }
        if !self.manufacturer.is_empty() {
            out += &format!("Manufacturer: {}\n", self.manufacturer);
        }
        out += &format!("HID collections (usage page:usage): {}\n", collections.join(", "));
        if !self.interfaces.is_empty() {
            let ifs: Vec<String> = self.interfaces.iter().map(i32::to_string).collect();
            out += &format!("USB interfaces: {}\n", ifs.join(", "));
        }
        out
    }
}

/// Devices in `api` with no interface that one of the `drivers` claims.
pub fn unknown_devices(api: &Api, drivers: &[&dyn Driver]) -> Vec<UnknownDevice> {
    let claimed: Vec<(u16, u16)> = api
        .devices()
        .filter(|d| drivers.iter().any(|drv| drv.claims(d)))
        .map(|d| (d.vendor_id, d.product_id))
        .collect();
    let mut groups: BTreeMap<(u16, u16), UnknownDevice> = BTreeMap::new();
    for d in api.devices() {
        if claimed.contains(&(d.vendor_id, d.product_id)) {
            continue;
        }
        let u = groups.entry((d.vendor_id, d.product_id)).or_insert_with(|| UnknownDevice {
            vid: d.vendor_id,
            pid: d.product_id,
            product: String::new(),
            manufacturer: String::new(),
            collections: Vec::new(),
            interfaces: Vec::new(),
        });
        if u.product.is_empty() {
            u.product = d.product.trim().to_string();
        }
        if u.manufacturer.is_empty() {
            u.manufacturer = d.manufacturer.trim().to_string();
        }
        if !u.collections.contains(&(d.usage_page, d.usage)) {
            u.collections.push((d.usage_page, d.usage));
        }
        if d.interface_number >= 0 && !u.interfaces.contains(&d.interface_number) {
            u.interfaces.push(d.interface_number);
        }
    }
    groups
        .into_values()
        .map(|mut u| {
            u.collections.sort();
            u.interfaces.sort();
            u
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(collections: &[(u16, u16)]) -> UnknownDevice {
        UnknownDevice {
            vid: 0x1532,
            pid: 0x0099,
            product: "Razer Viper".into(),
            manufacturer: "Razer".into(),
            collections: collections.to_vec(),
            interfaces: vec![0, 1],
        }
    }

    #[test]
    fn configurable_needs_input_and_vendor_interfaces() {
        assert!(device(&[(0x0001, 0x0002), (0xFF00, 0x0001)]).is_configurable());
        assert!(!device(&[(0x0001, 0x0002), (0x000C, 0x0001)]).is_configurable());
        assert!(!device(&[(0xFF00, 0x0001)]).is_configurable());
    }

    #[test]
    fn names_and_report() {
        let d = device(&[(0x0001, 0x0002)]);
        assert_eq!(d.id(), "1532:0099");
        assert_eq!(d.name(), "Razer Viper");
        assert!(d.report().contains("HID collections (usage page:usage): 0001:0002"));
        let nameless = UnknownDevice { product: String::new(), manufacturer: String::new(), ..d };
        assert_eq!(nameless.name(), "Unknown device 1532:0099");
    }
}
