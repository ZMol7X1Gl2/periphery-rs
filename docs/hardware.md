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
| Logitech | Other HID++ 2.0 mice | Receiver or USB | 🟡 | 🟡 | |
| Ajazz | AJ179 PRO / APEX / MC PRO, AJ159 PRO / APEX | Wired (`3151:4026` …) or receiver | 🟡 | 🟡 | Protocol recovered from the qmk.top web driver |
| AULA | F75 | USB (`258A:010C`) | 🟡 | 🟡 | Protocol from community re-implementations checked on an F75 |
| AULA | F87 / F87 PRO / F99 / F99 Pro / H108 | USB (`258A:010C`) | 🟡 | 🟡 | Same protocol; brightness 0-4 instead of 0-9 |
| AULA | F75 and siblings | 2.4 GHz dongle (`3554:FA09`) / Bluetooth | ❌ | ❌ | Different 20-byte transport |

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
| Backup / restore | ✅ |
| Notice when a paired mouse is asleep | ❌ (the mouse is left out of the list) |

### Ajazz (Rongyuan firmware)

| Feature | Status |
|---|---|
| Model detection, firmware, battery (receiver) | 🟡 |
| Onboard profiles | 🟡 |
| Polling rate | 🟡 |
| DPI stages and colours, active stage | 🟡 |
| Lift-off distance, angle snapping, ripple control | 🟡 |
| Buttons | 🟡 |
| Factory reset | 🟡 |
| Backup / restore | 🟡 |

Open questions for the first test: whether the `D3`/`D4` replies echo the
command byte, the button matrix (`D0`) reply offset, and the receiver's
product id. `periphery-cli diag` prints the raw replies.

### AULA

| Feature | Status |
|---|---|
| Model detection (identity bytes) | 🟡 |
| Lighting effect, brightness, speed, colour | 🟡 |
| Backlight sleep timer | 🟡 |
| Factory reset | 🟡 |
| Backup / restore | 🟡 |
| Keymap | ❌ (community sources disagree on the format) |
| Per-key RGB | ❌ (community sources disagree on the format) |
| Debounce | ❌ (offset unconfirmed) |
| 2.4 GHz dongle and Bluetooth | ❌ |

Open questions for the first test: whether the identity is read from the
right collection on Windows (the keyboard has several `0xFF00` collections),
which direction the speed levels run, and whether every effect's settings sit
at `0x3A + 2 × (effect − 1)` (only effect 1 has been confirmed).
`periphery-cli diag` prints the identity and the raw lighting page.
