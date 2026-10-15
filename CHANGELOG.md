# Changelog

All notable changes for people using Periphery, newest first. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/).

Add your change under **Unreleased** in the same commit, in the section
that fits: Added, Changed, Deprecated, Removed, Fixed or Security. The
release workflow turns Unreleased into the new version.

## [Unreleased]

### Added

- ROCCAT Kone Aimo and Kone Aimo Remastered, on the desktop and in the
  browser: onboard profiles, polling rate, DPI stages, lighting (effect,
  speed, brightness, zone colours), buttons and the Easy-Shift layer,
  including keyboard shortcuts. Not yet tested on a real mouse.

### Changed

- The Windows download includes this changelog.

## [0.2.1] - 2026-10-15

### Added

- [docs/hardware.md](https://github.com/ZMol7X1Gl2/periphery-rs/blob/master/docs/hardware.md) lists which devices and features have
  been tested on real hardware.

### Fixed

- Logitech: changing a profile's default DPI stage now takes effect at once,
  instead of only after the profile reloads.

## [0.2.0] - 2026-10-15

### Added

- Periphery runs in the browser: open
  https://zmol7x1gl2.github.io/periphery-rs/ in Chrome, Edge or Opera and
  connect your devices. Nothing to install.
- Ajazz mice on Rongyuan firmware (AJ179 PRO / APEX / MC PRO, AJ159 PRO /
  APEX), wired or through their receiver: battery, profiles, polling rate,
  DPI stages and colours, lift-off distance, angle snapping, ripple control,
  buttons and factory reset. Not yet tested on a real mouse.

### Changed

- The protocol libraries (`scyrox`, `logitech-hidpp`, `qmk-via`,
  `rongyuan`) have async APIs over the new `hid-transport` crate, so the
  same code runs on the desktop and in the browser.

## [0.1.1-rc.2] - 2026-10-13

### Fixed

- Logitech: crash when reading the DPI list of some older mice.

## [0.1.1-rc.1] - 2026-10-13

### Added

- Logitech: live DPI and polling rate for newer mice such as the PRO X
  Superlight 2.
- `periphery-cli diag` prints read-only device details to attach to bug
  reports.

### Changed

- Logitech: mice that only have their factory profiles show them read-only,
  and one failing read no longer hides the rest of the settings.

## [0.1.0] - 2026-10-13

### Added

- Desktop app and command-line tool for Scyrox mice, QMK/VIA keyboards
  (with Keychron's Hall-effect, per-key RGB and other extensions) and
  Logitech HID++ 2.0 mice.
- Backup and restore of every device to a JSON file.

[Unreleased]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.1.1-rc.2...v0.2.0
[0.1.1-rc.2]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.1.1-rc.1...v0.1.1-rc.2
[0.1.1-rc.1]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.1.0...v0.1.1-rc.1
[0.1.0]: https://github.com/ZMol7X1Gl2/periphery-rs/releases/tag/v0.1.0
