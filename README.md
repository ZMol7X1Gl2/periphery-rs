# Periphery

One app to configure mice and keyboards from different brands, instead of each
vendor's web driver. The UI knows nothing about brands: every driver describes
its settings as data and the GUI/CLI render whatever a device reports.

Currently supported:

- **Scyrox** mice: DPI stages, polling rate, sensor options, buttons (incl. fire,
  combo, media keys, macros), profiles, sleep, pairing.
- **Logitech** HID++ 2.0 mice (tested: PRO X Superlight and Superlight 2):
  battery, onboard/software mode, live DPI and polling rate, onboard profile
  editing (DPI stages, polling rate, buttons, name, reset to factory).
- **Ajazz** mice on Rongyuan firmware (AJ179 PRO / APEX / MC PRO, AJ159 PRO /
  APEX), wired or through their receiver: battery, profiles, polling rate, DPI
  stages and colours, lift-off distance, angle snapping, ripple control,
  buttons, factory reset. Not yet tested on a real mouse.
- **QMK/VIA** keyboards of any brand: keymap, layers, lighting menus, macros.
- **Keychron** extensions on top of VIA: magnetic-switch (Hall-effect) actuation
  and rapid trigger, global and per key; snap tap; per-key RGB; sleep timers;
  debounce, polling rate, NKRO and more where the firmware supports them.

Every device can be backed up to JSON and restored; restore writes only the
values that differ.

See **[docs/hardware.md](docs/hardware.md)** for which devices and features
have been tested on real hardware.

## Crates

| Crate | Kind | Purpose |
|---|---|---|
| [`scyrox`](crates/scyrox) | library | Scyrox mouse protocol (HID transport, flash map, encodings). No Periphery dependency. |
| [`qmk-via`](crates/qmk-via) | library | QMK/VIA raw-HID protocol, keycodes, layout definitions, macros, menus, and Keychron extensions. No Periphery dependency. |
| [`logitech-hidpp`](crates/logitech-hidpp) | library | Logitech HID++ 2.0: feature discovery, battery, DPI, report rate, onboard profiles (format + CRC). No Periphery dependency. |
| [`rongyuan`](crates/rongyuan) | library | Rongyuan firmware protocol (feature-report transport, receiver relay, PAN1080 mouse commands, Ajazz model list). No Periphery dependency. |
| [`periphery-core`](crates/periphery-core) | library | Brand-neutral model: `Device`/`Driver` traits, setting descriptors, keymaps, per-key properties, key-pair lists, backups. |
| [`periphery-scyrox`](crates/periphery-scyrox) | adapter | Exposes `scyrox` through the core model. |
| [`periphery-via`](crates/periphery-via) | adapter | Exposes `qmk-via` through the core model. |
| [`periphery-logitech`](crates/periphery-logitech) | adapter | Exposes `logitech-hidpp` through the core model. |
| [`periphery-rongyuan`](crates/periphery-rongyuan) | adapter | Exposes `rongyuan` through the core model. |
| [`hid-transport`](crates/hid-transport) | library | Async HID with one API over hidapi (desktop) and WebHID (browser). |
| [`periphery`](crates/app) | app | GUI (`periphery`) and CLI (`periphery-cli`); the GUI is shared with the web app. |
| [`periphery-web`](crates/web) | app | The GUI in the browser, published to GitHub Pages. |

The protocol crates can be used on their own. Their I/O is async over
[`hid-transport`](crates/hid-transport), which uses hidapi on the desktop and
WebHID in the browser:

```rust
let api = hid_transport::Api::new().await?;
let mouse = scyrox::device::Mouse::open(&api).await?;
println!("{:?}", mouse.read_settings().await?);
```

Adding a brand = a protocol crate + an adapter implementing
`periphery_core::Driver` and `Device`, registered in `crates/app/src/lib.rs`.
See **[docs/adding-a-device.md](docs/adding-a-device.md)** for the full guide
and [CONTRIBUTING.md](CONTRIBUTING.md) for contribution rules.

## Running

### In the browser

Open the GitHub Pages site in Chrome, Edge or Opera on a desktop (Firefox
and Safari have no WebHID), click **Connect a device…** and pick your
devices. Chrome remembers the choice for next time.

- `master`: https://zmol7x1gl2.github.io/periphery-rs/
- any other branch: https://zmol7x1gl2.github.io/periphery-rs/<branch>/
  (lowercased, other characters than `a-z 0-9 . - _` become `-`)

Locally: `cargo install trunk`, `rustup target add wasm32-unknown-unknown`,
then `trunk serve` in `crates/web` and open http://127.0.0.1:8080/.

On Linux the browser needs the same udev rule as the desktop app.

### On the desktop

```
cargo run --release --bin periphery                 # GUI
cargo run --release --bin periphery-cli -- list     # CLI
periphery-cli show <n> | keymap <n> [layer] | set <n> <id> <value>
periphery-cli setprop <n> <id> <value|default> <row,col>...
periphery-cli backup <n> <file.json> | restore <n> <file.json> [--dry-run]
```

GUI options: `--device <name>` preselects a device, `--tab keymap[:<setting>]`
opens the keymap tab (optionally on a per-key setting such as `actuation`).

Keychron layout definitions are downloaded once from launcher.keychron.com and
cached in `%APPDATA%\periphery\definitions`. For other VIA keyboards, place a
definition (pre-parsed `layouts.keys` format) there as `{vid}_{pid}.json`.

## Status

Tested and untested devices and features are tracked in
[docs/hardware.md](docs/hardware.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
