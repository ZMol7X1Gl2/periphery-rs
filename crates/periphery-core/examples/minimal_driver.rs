//! A complete, minimal Periphery driver for an imaginary mouse, used by
//! `docs/adding-a-device.md`. It talks to no hardware: the "device" is a
//! struct in memory, so the example runs anywhere:
//!
//! ```text
//! cargo run -p periphery-core --example minimal_driver
//! ```

use anyhow::{Result, bail};
use periphery_core::hid::{Api, Filter};
use periphery_core::{Device, DeviceInfo, DeviceKind, Driver, Group, Kind, Setting, Snapshot, Value, async_trait};

// ------------------------------------------------------------------------
// In a real driver this part lives in a protocol crate (e.g. `scyrox`,
// `logitech-hidpp`): HID transport, command ids, encodings. It must not
// depend on periphery-core. Its I/O methods are `async fn`s over
// `hid_transport::Device`, so it also runs in the browser.

struct FakeMouse {
    dpi: u16,
    polling_hz: u16,
    angle_snap: bool,
}

impl FakeMouse {
    fn set_dpi(&mut self, dpi: u16) -> Result<()> {
        if !(100..=16000).contains(&dpi) || dpi % 50 != 0 {
            bail!("DPI must be 100-16000 in steps of 50");
        }
        self.dpi = dpi;
        Ok(())
    }
}

// ------------------------------------------------------------------------
// The adapter: describes the device's settings as data and applies changes.

pub struct ExampleDriver;

#[async_trait(?Send)]
impl Driver for ExampleDriver {
    fn name(&self) -> &'static str {
        "example"
    }

    /// The interfaces this driver uses; the browser's device picker lists
    /// only devices matching one of them.
    fn filters(&self) -> Vec<Filter> {
        vec![Filter { vendor_id: Some(0x1234), usage_page: Some(0xFF00), ..Filter::default() }]
    }

    /// Real drivers filter `api.devices()` with their filters, open the
    /// matching interfaces with `api.open(info).await` and return one
    /// `Device` per product. Return an empty list when nothing matches;
    /// return `Err` only for a device that is present but failed to open.
    async fn discover(&self, _api: &Api) -> Vec<Result<Box<dyn Device>>> {
        let mouse = FakeMouse { dpi: 800, polling_hz: 1000, angle_snap: false };
        let info = DeviceInfo { brand: "Example".into(), model: "Mouse 1".into(), kind: DeviceKind::Mouse, vid: 0x1234, pid: 0x5678 };
        vec![Ok(Box::new(ExampleDevice { mouse, info }))]
    }
}

struct ExampleDevice {
    mouse: FakeMouse,
    info: DeviceInfo,
}

#[async_trait(?Send)]
impl Device for ExampleDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// Read everything from the device and describe it. Setting ids are
    /// stable strings your `set` understands; they also end up in backups.
    async fn snapshot(&mut self) -> Result<Snapshot> {
        let mut out = Snapshot::default();
        out.status.push(("Firmware".into(), "1.0".into()));

        let mut g = Group::new("Performance");
        g.push(Setting::new(
            "dpi",
            "DPI",
            Kind::range_unit(100, 16000, 50, 1.0, " DPI"),
            Value::Int(self.mouse.dpi as i64),
        ));
        g.push(Setting::new(
            "rate",
            "Polling rate",
            Kind::choice([(125, "125 Hz"), (500, "500 Hz"), (1000, "1000 Hz")]),
            Value::Int(self.mouse.polling_hz as i64),
        ));
        g.push(Setting::toggle("angle_snap", "Angle snapping", self.mouse.angle_snap).help("Straightens diagonal movement"));
        out.groups.push(g);

        let mut power = Group::new("Maintenance");
        power.push(Setting::action("reset", "Factory reset", Some("Reset every setting?")));
        out.groups.push(power);
        Ok(out)
    }

    /// Apply one setting. Validate before writing; return errors the user can read.
    async fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        match id {
            "dpi" => self.mouse.set_dpi(value.int()? as u16),
            "rate" => {
                self.mouse.polling_hz = value.int()? as u16;
                Ok(())
            }
            "angle_snap" => {
                self.mouse.angle_snap = value.bool()?;
                Ok(())
            }
            "reset" => {
                self.mouse = FakeMouse { dpi: 800, polling_hz: 1000, angle_snap: false };
                Ok(())
            }
            _ => bail!("unknown setting {id}"),
        }
    }
}

fn main() -> Result<()> {
    // Desktop builds drive the async code with a simple executor.
    pollster::block_on(run())
}

async fn run() -> Result<()> {
    let api = Api::new().await?;
    for dev in ExampleDriver.discover(&api).await {
        let mut dev = dev?;
        dev.set("dpi", &Value::Int(1600)).await?;
        let snap = dev.snapshot().await?;
        println!("{} → {:?}", dev.info().title(), snap.find("dpi").map(|s| &s.value));
        // Backups work for every driver without extra code:
        let backup = periphery_core::backup::Backup::capture(dev.as_mut()).await?;
        println!("backup has {} settings", backup.settings.len());
    }
    Ok(())
}
