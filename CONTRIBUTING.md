# Contributing to Periphery

Thanks for helping! The most valuable contributions are drivers for devices we
don't support yet — see **[docs/adding-a-device.md](docs/adding-a-device.md)**
for a step-by-step guide.

## Setup

- Rust stable (edition 2024), `cargo`.
- **Windows**: nothing else needed.
- **Linux**: `libudev-dev` (for `hidapi`) and a udev rule giving your user
  access to the device's `hidraw` node.

```
cargo build --workspace --all-targets
cargo test --workspace
cargo run --release --bin periphery            # GUI
cargo run --release --bin periphery-cli -- list
```

## Ground rules

- **Safety first.** Users trust this tool with their hardware. Only send
  commands whose meaning you've confirmed; probe read-only first; back up
  before writing; read back what you write to flash. Never guess a write
  format — show the setting read-only until it's confirmed.
- **Keep brands out of the UI.** Device knowledge belongs in drivers. If the
  UI needs a new kind of control, extend `periphery-core` generically.
- **Protocol crates stay independent.** `scyrox`, `qmk-via`,
  `logitech-hidpp` and new ones must not depend on `periphery-core`.
- **Test with real bytes.** Encoders and decoders get unit tests built from
  data read off actual hardware.
- **Say what's untested.** If you couldn't verify something on hardware,
  write it in the PR and in the setting's help text, and mark it 🟡 in
  [docs/hardware.md](docs/hardware.md). Tested something? Mark it ✅ there.
- Match the surrounding code style and run `cargo fmt` on files you touch.

## Pull requests

Please include:

- The device(s) you tested: model, connection (USB / receiver / Bluetooth)
  and firmware version.
- What you verified on hardware, and what you couldn't.
- Where the protocol knowledge came from (web driver, vendor app, public
  docs, USB capture).
- Updated `README.md` (supported devices, crate table) when adding a driver.

CI builds and tests the workspace on Windows for every pull request.

## Changelog and releases

Describe user-visible changes in [CHANGELOG.md](CHANGELOG.md) under
**Unreleased**, in the same pull request, using the section that fits
(Added, Changed, Deprecated, Removed, Fixed, Security). Write for people
using Periphery, not for developers: what changed for them, not how.
Internal changes (refactors, CI, tests) need no entry.

To release `master`, run the **Release** workflow from the Actions tab, or:

```
gh workflow run release.yml -f bump=patch            # or minor / major
gh workflow run release.yml -f version=0.3.0-rc.1    # exact; suffix = pre-release
```

It turns Unreleased into the new version, bumps the version, commits,
tags, builds the Windows zip and publishes the GitHub release with that
changelog section as its notes. It refuses to run while Unreleased is
empty.

## License

By contributing, you agree that your contributions are dual-licensed under
the MIT and Apache-2.0 licenses, like the rest of the project (see
[README.md](README.md#license)).
