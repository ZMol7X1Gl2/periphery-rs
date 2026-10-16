# Hardware support

What each driver implements, and what has been confirmed on a real device.
Please update this file when you test something (see
[CONTRIBUTING.md](../CONTRIBUTING.md)); include the firmware version when you
know it.

- ✅ tested on real hardware
- 🟡 implemented, not tested on real hardware yet
- ❌ not implemented

"Desktop" covers the GUI and `periphery-cli`; "Web" is the browser app
(Chrome, Edge or Opera over WebHID). Both run the same driver code.

## Devices

| Brand | Device | Connection | Desktop | Web | Notes |
|---|---|---|---|---|---|
| Scyrox | Mouse with PAW3950 (cid 62) | 8K receiver (`3554:F5F7`) | ✅ | ✅ | Receiver fw v2.16, mouse fw v2.20 |
| Scyrox | Other Scyrox mice (PIDs `F5F4`, `F517`, `F5F6`, `F515`) | Receiver or wired | 🟡 | 🟡 | Same protocol; PAW3955 DPI encoding untested |
| Keychron | K2 HE ANSI RGB (`3434:0E20`) | USB | ✅ | ✅ | Firmware 1.2.1, Hall-effect command version 4 |
| Keychron | Other Keychron boards | USB / 2.4 GHz / Bluetooth | 🟡 | 🟡 | Layout downloaded from the Keychron launcher |
| QMK/VIA | Any VIA-enabled keyboard | USB | 🟡 | 🟡 | Needs a layout definition file for the keymap |
| Logitech | PRO X Superlight | Lightspeed receiver (`046D:C547`) | ✅ | 🟡 | Firmware MPM25.01, HID++ 4.2 |
| Logitech | PRO X Superlight 2 | Lightspeed receiver | ✅ | 🟡 | Software (live) mode confirmed working since 0.1.1-rc.2 |
| Logitech | PRO 2 LIGHTSPEED | Lightspeed receiver | 🟡 | 🟡 | Lighting untested |
| Logitech | Other HID++ 2.0 mice | Receiver or USB | 🟡 | 🟡 | |
| Ajazz | AJ179 PRO / APEX / MC PRO, AJ159 PRO / APEX | Wired (`3151:4026` …) or receiver | 🟡 | 🟡 | Protocol recovered from the qmk.top web driver |
| ROCCAT | Kone Aimo Remastered / 16K (`1E7D:2E2C`) | USB | ✅ | ❌ | Desktop on Windows: DPI and button remapping confirmed (0.3.0-rc.1). Web: Chrome, Edge and Arc on Windows hide the settings interface (tested with a 16K) |
| ROCCAT | Kone Aimo (`1E7D:2E27`) | USB | 🟡 | ❌ | Assumed to share the Remastered's layout (libratbag PR #1883 treats them alike) |

## Features

### Scyrox

| Feature | Status |
|---|---|
| Battery, firmware, connection | ✅ |
| Polling rate, DPI stages and colours, active stage | ✅ |
| Lift-off distance, motion sync, angle snapping, ripple, competition mode, 20K FPS, angle tune, debounce | ✅ |
| Buttons: simple actions, fire, combo keys, media keys, macros | ✅ |
| Onboard profiles | ✅ |
| Sleep timer, long-range mode | ✅ |
| Receiver pairing | ✅ |
| Factory reset | ✅ |
| Backup / restore | ✅ |

### Keychron and QMK/VIA keyboards

| Feature | Status |
|---|---|
| Firmware, connection, VIA protocol | ✅ |
| Keymap (all layers) | ✅ |
| Lighting menus (VIA) | ✅ |
| Macros | ✅ |
| Per-key Hall-effect actuation and rapid trigger | ✅ |
| Reset per-key travel to global | ✅ |
| Global Hall-effect actuation and rapid trigger | 🟡 |
| Hall-effect profile switching | 🟡 |
| Snap tap (SOCD) pairs | 🟡 (reading ✅) |
| Per-key RGB colours | ✅ |
| Sleep timers | ✅ (reading) / 🟡 (writing) |
| Debounce, polling rate, NKRO, performance mode, DIP switch | ✅ (reading) / 🟡 (writing) |
| Backup / restore | ✅ |
| DKS, toggle, turbo, DKR editing | ❌ (shown read-only) |
| Hall-effect calibration | ❌ |
| Hall-effect profile rename | ❌ |
| Keychron mice | ❌ |

### Logitech (HID++ 2.0)

| Feature | Status |
|---|---|
| Battery, firmware, name | ✅ |
| Onboard / software mode switch | ✅ |
| Live DPI and polling rate (0x2201 / 0x8060) | ✅ |
| Live DPI and polling rate, extended (0x2202 / 0x8061) | ✅ (Superlight 2) |
| Onboard profile editing: DPI stages, polling rate, buttons, name | ✅ |
| Reset profile to factory | ✅ |
| Lighting in onboard profiles: effect, colour, cycle time, brightness | 🟡 |
| Live lighting (0x8070 / 0x8071) | 🟡 |
| Backup / restore | ✅ |
| Notice when a paired mouse is asleep | ❌ (the mouse is left out of the list) |

### Ajazz (Rongyuan firmware)

| Feature | Status |
|---|---|
| Model detection, firmware, battery (receiver) | 🟡 |
| Onboard profiles | 🟡 |
| Polling rate | 🟡 |
| DPI values per stage (AJ179 PRO, web app) | ✅ |
| Stage count, stage colours | 🟡 |
| Active stage | Read only: the AJ179 PRO ignores it on write |
| Lift-off distance, angle snapping, ripple control | 🟡 |
| Buttons | 🟡 |
| Factory reset | 🟡 |
| Backup / restore | 🟡 |

First hardware report (a friend's AJ179 PRO in the web app): editing a
stage's DPI works, so the `D4` reply echoes the command byte. Writing the
active stage (byte 2 of `54`) changes nothing. Still open: the button
matrix (`D0`) reply offset and the receiver's product id. `periphery-cli diag` prints the raw replies.

### ROCCAT Kone Aimo

| Feature | Status |
|---|---|
| Onboard profiles (switch active profile) | 🟡 |
| Polling rate | 🟡 |
| DPI stages (X and Y set together), active stage | ✅ (Kone Aimo 16K) |
| Lighting: effect, speed, brightness, zone colours | 🟡 (0.3.0-rc.1 had no effect while a Swarm theme was active; the fix is not tested yet) |
| Buttons and Easy-Shift layer | ✅ (Kone Aimo 16K) |
| Keyboard shortcuts on buttons | 🟡 |
| Backup / restore | 🟡 |
| Macros | ❌ (shown as "Macro", kept as is) |
| Sensitivity, angle snapping, live (non-saved) lighting | ❌ |

The web app can't configure these mice: on a Kone Aimo 16K, Chrome, Edge and
Arc on Windows listed the mouse in their picker (which matches usage page
`0x0B`) but removed that collection's reports before the page got the device;
only the `0x0A` and `0x0C` collections reached it, and feature reports `04`/`05`
failed with "Invalid feature report ID" in the device log. Chromium removes
reports it considers protected at that step; its built-in blocklist has no
ROCCAT entry, so the rule that matches is not known yet.

Things to check on the first desktop test: that the device is found on Windows (the
driver opens the usage page `0x0B` collection on interface 0, as OpenRGB
does), that reads work through hidapi on Windows (they fall back to a
1100-byte buffer if an exact-size read is refused), and that a written value
reads back. `periphery-cli diag` prints the HID collections and raw reports.
