# Adding a device

This guide walks through adding support for a new mouse, keyboard or headset,
from finding out how the vendor's software talks to it, to a tested driver in
the app. A complete minimal driver lives in
[`crates/periphery-core/examples/minimal_driver.rs`](../crates/periphery-core/examples/minimal_driver.rs);
run it with `cargo run -p periphery-core --example minimal_driver`.

## 0. Is it already supported?

Check these first — many devices need no new code:

| Your device | What to do |
|---|---|
| Any **QMK/VIA keyboard** | It's detected automatically (usage page `0xFF60`). For the keymap view, put its VIA definition (pre-parsed `layouts.keys` format) into `%APPDATA%\periphery\definitions\{vid}_{pid}.json`. |
| A **Keychron** keyboard | Detected and its layout is downloaded from Keychron automatically. |
| A **Logitech** mouse with HID++ 2.0 | Detected automatically through its receiver or USB cable. Features it lacks are simply not shown. |
| An **Ajazz** (or other Rongyuan-firmware) PAN1080 mouse | If `periphery-cli show` says its model id isn't in the list, add it to `MODELS` in `crates/rongyuan/src/models.rs` (values from the device list in the qmk.top/v4 web driver), and its USB ids to `MOUSE_PIDS` / `RECEIVERS` if it isn't found at all. |
| A **ROCCAT Kone Aimo** variant with another USB id | Add it to `MODELS` in `crates/roccat/src/kone_aimo.rs` with its DPI range. |
| Another **Scyrox** model | Add its USB product ids to `PRODUCT_IDS` in `crates/scyrox/src/protocol.rs`, and its sensor to `Sensor::from_mid` if new. |

Run `periphery-cli list` to see what's detected. If your device shows up, you
may only need to fill gaps in an existing driver (step 6 still applies).

## How the pieces fit

```
crates/
  <brand>/             protocol library: HID transport, command ids, encodings
                       (depends only on hid-transport/anyhow — usable without Periphery)
  hid-transport/       async HID: hidapi on the desktop, WebHID in the browser
  periphery-<brand>/   adapter: implements periphery_core::{Driver, Device}
  periphery-core/      the brand-neutral model the UI renders
  app/                 GUI + CLI; lists every driver in src/lib.rs
  web/                 the same GUI in the browser (GitHub Pages)
```

All device I/O is `async`, so one driver runs in the desktop app, the CLI
and the web app. On the desktop the async calls simply block a worker
thread; in the browser they wait on WebHID promises. Never call
`std::thread::sleep` or `std::time::Instant` in driver code (both fail in the
browser): use `hid_transport::sleep(..).await` and `hid_transport::Instant`.

The UI never knows which brand it shows. A driver *describes* its settings as
data (`Setting` with a `Kind`: toggle, range, choice, colour, text, action,
info) and receives changes by setting id. Keyboards can also expose a keymap,
per-key properties (actuation, colours) and key-pair lists (snap tap). Backup
and restore work for every driver automatically.

## 1. Find the protocol

Never guess byte formats: find where the vendor software builds them.

1. **Vendor web driver (WebHID).** Many brands configure devices from a web
   page. Download its JavaScript, pretty-print it
   (`npx prettier --parser babel bundle.js`) and search for `navigator.hid`,
   `requestDevice` (the `filters` give vendor id / usage page),
   `sendReport`, `sendFeatureReport` and `inputreport`. Command enums and
   per-device JSON configs (`cfg.json`-style files) are usually nearby.
   Scyrox and Keychron were done this way.
2. **Desktop app.** Electron apps keep their code in `resources/app.asar`.
   Device packages, JSON manifests and default configurations are often
   readable files (G HUB keeps them in `C:\ProgramData\LGHUB\depots`).
   Strings in native agents (`grep -a`) confirm which features are used.
3. **Public documentation.** Before decompiling anything, check
   [libratbag](https://github.com/libratbag/libratbag),
   [Solaar](https://github.com/pwr-Solaar/Solaar),
   [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) and
   [QMK](https://docs.qmk.fm/) — Logitech HID++ was implemented from these and
   confirmed on the device.
4. **USB capture** (Wireshark + USBPcap) while the vendor app changes one
   setting at a time, as a last resort or to confirm an ambiguity.

Write down every command with its source (file, function) — you'll need it in
doc comments and when something doesn't behave as expected.

## 2. Probe the device read-only

Make a throwaway Rust project with `hidapi` (or `hid-transport`) and talk to the real device
before writing the driver:

- List the interfaces (`vendor_id`, `product_id`, `usage_page`, `usage`).
  Configuration usually sits on a vendor usage page (`0xFF00`-`0xFFFF`).
- Send **only commands you know are reads** (get version, get setting,
  read memory) and print the replies.
- Decode the replies by hand and check them against what the vendor app
  shows. Save interesting replies — they become your unit-test fixtures.

**Do not send commands you haven't identified.** The same id can mean
"get" on one firmware and "set" on another (Keychron's misc sub-command 18
is "get NKRO" on some boards and "set performance mode" on others). Respect
feature flags the device reports before probing optional commands.

## 3. Write the protocol crate

`crates/<brand>/` — a library anyone can use without Periphery.

- Dependencies: `hid-transport`, `anyhow` (plus `serde` only if you parse
  files). **No dependency on `periphery-core`.**
- I/O methods are `async fn`s over `hid_transport::Device`. Its calls have
  the shape of `hidapi`'s (`write`, `read_timeout`, `send_feature_report`,
  `get_feature_report`) with the same byte layout: the first byte written is
  the report id, and a read starts with the report id only for numbered
  reports.
- Use the workspace metadata in `Cargo.toml`:
  ```toml
  [package]
  name = "<brand>"
  description = "..."
  version.workspace = true
  edition.workspace = true
  license.workspace = true
  repository.workspace = true
  ```
- Typical modules: `transport` (framing, request/reply matching with a
  timeout, retries), command constants, and pure encode/decode functions.
- Document the wire format in doc comments (byte tables like those in
  `crates/logitech-hidpp/src/profile.rs`).
- **Unit-test every encoding with real bytes from your device** — see
  `travel_from_k2_he` in `qmk-via` or `parses_real_profile` in
  `logitech-hidpp`. Round-trip tests (decode → encode == original) catch most
  mistakes, including checksum algorithms.
- Keep unknown bytes when rewriting structured data (profiles, flash
  sectors): read, modify the fields you understand, write back.

## 4. Write the adapter

`crates/periphery-<brand>/` depends on your protocol crate and
`periphery-core`, and implements two traits:

```rust
#[async_trait(?Send)]
impl Driver for MyDriver {
    fn name(&self) -> &'static str;
    fn filters(&self) -> Vec<Filter>;                   // what the browser may offer
    async fn discover(&self, api: &Api) -> Vec<Result<Box<dyn Device>>>;
}

#[async_trait(?Send)]
impl Device for MyDevice {
    fn info(&self) -> &DeviceInfo;
    async fn snapshot(&mut self) -> Result<Snapshot>;    // read + describe
    async fn set(&mut self, id: &str, value: &Value) -> Result<()>;
    // optional:
    async fn set_key(&mut self, layer: u8, row: u8, col: u8, code: u16) -> Result<()>;
    async fn set_key_property(&mut self, id: &str, keys: &[(u8, u8)], value: Option<&Value>) -> Result<()>;
    async fn set_key_pairs(&mut self, id: &str, pairs: &[KeyPair]) -> Result<()>;
    async fn poll_changes(&mut self) -> Result<bool>;    // on-device changes
}
```

`async_trait`, `hid::{Api, Filter}` and the rest come from `periphery_core`.

Guidelines:

- **`filters`**: the vendor id / product ids / usage page of the interfaces
  you use. The browser only lets a page see devices the user picked, and its
  picker lists only devices matching some driver's filters.
- **`claims`** (optional): whether your driver handles an interface even
  when nothing could be opened (e.g. a receiver whose mouse sleeps).
  Defaults to your filters; override it when your filters also match
  devices you don't support, or they'd never be offered for a support
  request.
- **`discover`**: filter `api.devices()` by vendor id and usage page (a
  `Filter` has `matches`), open with `api.open(info).await`; return nothing
  when your hardware isn't present. Return `Err` only for a device that
  exists but failed to open.
- **`snapshot`**: read everything, then describe it. Group related settings
  (`Group::new("Sensor")`). Put read-only facts (battery, firmware,
  connection) in `status`. If the device can't be configured right now
  (asleep, offline), return a `notice` instead of failing.
- **Setting ids** are stable strings (`"dpi.2"`, `"he.actuation"`). They are
  saved in backups, so don't rename them casually. An id ending in
  `.profile` (or exactly `profile`) is restored before everything else —
  use that for profile selectors.
- **Choose the right `Kind`**: `Kind::range_unit(min, max, step, scale,
  unit)` shows `value × scale` (e.g. 0.1 mm steps with `scale = 0.1`);
  `Kind::choice` for fixed options; `Kind::Text` for names and macros;
  `Kind::Action { confirm }` for one-shot commands (reset, pair).
- **Hide what the device doesn't support** instead of showing controls that
  fail. Use feature flags or a probe that tolerates "no reply".
- **`set`**: validate and return readable errors. For commands that carry
  several fields, keep the last-read values and send them all. For flash or
  EEPROM writes, read back and compare.
- **Keyboards**: fill `Snapshot::keymap` (layout from the definition,
  keycodes per layer, a keycode catalog). Per-key values go in
  `Keymap::properties`; pair lists (snap tap / SOCD) in `pair_lists`.
- Add an `Info` setting or `notice` for features you can read but not yet
  write, rather than guessing a write format.

## 5. Register the driver

1. `crates/app/Cargo.toml`: add `periphery-<brand> = { path = "../periphery-<brand>" }`.
2. `crates/app/src/lib.rs`: add your driver to `drivers()` (and bump the
   array length).

New crates in `crates/` join the workspace automatically.

## 6. Test on real hardware

Work through this in order and keep the CLI output for your pull request:

1. `periphery-cli list` — the device appears once, with a sensible name.
2. `periphery-cli show <n>` — every value matches the vendor app.
3. `periphery-cli backup <n> before.json` — **always before the first write.**
4. `periphery-cli restore <n> before.json --dry-run` — reports 0 changes
   (otherwise a value doesn't read back the way it's described).
5. Change one value at a time (`set`, `setprop`, `setkey`), `show` again,
   and confirm the device behaves accordingly.
6. `periphery-cli restore <n> before.json` and dry-run again: 0 changes.
7. Open the GUI (`periphery --device <name>`) and check the layout.

Avoid writing settings you can't easily undo (firmware, calibration, pairing)
until the basics are proven. Tell users in the UI help text which functions
are unverified.

## 7. Documentation and pull request

- Add the device to the "Currently supported" list and the crate table in
  `README.md`.
- In the PR description, name the exact model and firmware you tested, list
  what you verified on hardware (and what you couldn't), and where the
  protocol knowledge came from.
- `cargo build --workspace --all-targets` and `cargo test --workspace` must
  pass; CI runs both on Windows.

See [CONTRIBUTING.md](../CONTRIBUTING.md) for general rules.
