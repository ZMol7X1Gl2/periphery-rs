# Changelog

All notable changes for people using Periphery, newest first. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/).

Add your change under **Unreleased** in the same commit, in the section
that fits: Added, Changed, Deprecated, Removed, Fixed or Security. The
release workflow turns Unreleased into the new version.

## [Unreleased]

### Added

- Devices Periphery doesn't support yet are listed under "Not supported
  yet", with a **Request support** button that checks for an existing
  request and opens a prefilled GitHub issue you can review before sending.
  `periphery-cli list` prints the same links.
- **Report a bug** next to errors: under a device that failed to open in
  the sidebar, and in the error banner. It opens a prefilled GitHub issue
  with the error, the device and its USB id, firmware and connection, and
  the Periphery version, which you review before sending.
  `periphery-cli` prints the same link with each warning. Keyboard errors
  now name the keyboard and its USB id.
- Web app: **Connect another device…** lets you pick any device, so you can
  request support for it.
- Logitech: lighting for mice with LEDs such as the PRO 2 LIGHTSPEED. Choose
  off, static colour, colour cycle or breathing, plus the colour, cycle time
  and brightness. The setting is stored in the onboard profile, or applies
  live in software mode.

### Changed

- A Keychron Link receiver is listed as a receiver instead of failing with
  "no reply to command 0x01". It passes typing on but not settings, so
  selecting it explains that the keyboard needs a USB cable to be
  configured.
- Long notices and errors wrap instead of running off the window.
- Ajazz mice: the active DPI stage is shown but can no longer be changed
  from Periphery. The AJ179 PRO ignores it, so trying to change it failed
  with "the mouse did not keep the DPI change". Use the DPI button on the
  mouse instead. `periphery-cli diag` now also prints the raw DPI reply.
- Every mouse now uses the same page, whatever the brand. The profile sits
  at the top, then tabs: DPI & polling, Buttons, Sensor, Lighting, and
  Power & device. Polling rates are a row of buttons. DPI stages are one
  table where you pick the active stage, set the DPI and the stage colour,
  and add or remove stages with − / +. Settings only one brand has (Scyrox
  competition mode, the ROCCAT Easy-Shift layer, Logitech's onboard
  profile while in software mode…) are on that brand's own tab.
- Buttons are assigned on a drawing of a mouse: click a button on it (or in
  the list below it), then pick its action from the buttons next to it,
  sorted into Mouse, Scroll, DPI, Media, Keyboard, Macro and Other tabs
  (the search box looks through all of them). Extra options such as macro text or
  shortcut keys appear under the chosen action. Only the buttons the mouse
  has are drawn: side, thumb, wheel tilt and scroll, top and underside
  buttons. The mouse is drawn low-poly, after a modern symmetric shape: a
  flat front, long main buttons inset from the shell, and the side buttons
  along the left edge. The drawing takes only the room it needs.
- New look: a darker theme with one accent colour, a sidebar listing each
  device with its kind and brand, the device's status (battery, firmware,
  connection) as chips under its name, pill-shaped tabs, rounded cards and
  coloured banners for notices and errors. Factory reset is a red button
  of its own, and action rows say Start… or Run instead of repeating
  their name.
- On/off settings are switches instead of checkboxes.
- Text is set in Inter, and the icons (device kinds, rescan, backup,
  restore, banners, links) are one consistent line-icon set instead of
  emoji that looked different on every system. Selected choices keep
  rounded corners.
- Periphery has a logo: a tile with a P and two pairs of stars. It's in the
  sidebar, the desktop app's window and taskbar icon, and the web app's
  browser tab and iOS home-screen icon.
- The colours follow the logo: black and off-white mark whatever is
  chosen (tabs, options, switches, the selected device and mouse button),
  the greys are warm instead of blue, and colour is kept for status:
  green, amber and red.

## [0.3.0-rc.2] - 2026-10-15

### Fixed

- Changing a ROCCAT Kone Aimo's lighting had no effect while a ROCCAT Swarm
  lighting theme was active on the mouse; it now switches the mouse to the
  colours set in Periphery. The lighting section shows which theme is active.

## [0.3.0-rc.1] - 2026-10-15

### Added

- ROCCAT Kone Aimo and Kone Aimo Remastered in the desktop app: onboard
  profiles, polling rate, DPI stages, lighting (effect, speed, brightness,
  zone colours), buttons and the Easy-Shift layer, including keyboard
  shortcuts. Not yet tested on a real mouse. The web app finds these mice
  but can't configure them, because Chrome and Edge hide their settings
  interface; it says so and points to the desktop app.

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

[Unreleased]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.3.0-rc.2...HEAD
[0.3.0-rc.2]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.3.0-rc.1...v0.3.0-rc.2
[0.3.0-rc.1]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.2.1...v0.3.0-rc.1
[0.2.1]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.1.1-rc.2...v0.2.0
[0.1.1-rc.2]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.1.1-rc.1...v0.1.1-rc.2
[0.1.1-rc.1]: https://github.com/ZMol7X1Gl2/periphery-rs/compare/v0.1.0...v0.1.1-rc.1
[0.1.0]: https://github.com/ZMol7X1Gl2/periphery-rs/releases/tag/v0.1.0
