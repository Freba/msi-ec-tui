# msi-ec-tui

A terminal UI for the [msi-ec](https://github.com/BeardOverflow/msi-ec) kernel
driver (MSI laptops). It turns the driver's sysfs interface into a friendly
dashboard: pick shift/fan modes, toggle cooler boost, tune battery charge
thresholds, control the keyboard backlight, and watch CPU/GPU temperatures and
fan speeds live.

```
Performance    Thermals    Battery    Input & LED    Info    ⚠ DEBUG
┌ Performance ──────────────────────────────────────────────
│▸ Shift mode            ◀ comfort ▶  (w)
│  Cooler boost          [ ON ]  (w)
┌ Thermals ─────────────────────────────────────────────────
│  CPU temperature        54 °C ▂▃▃▄▅
│  CPU fan                35 %  ▂▂▃▃▃
```

## Features

- **Zero config** — attributes are discovered dynamically, so the same binary
  works with the DKMS driver, the upstream in-tree driver, and firmware that
  exposes only a subset of features. Missing features are shown as
  *not supported* instead of failing.
- **Live sensors** — CPU/GPU temperature and fan speed are polled every second
  and rendered as gauges with sparkline history.
- **Battery care** — sliders for `charge_control_start/end_threshold`
  (charge limits) and the super-battery toggle.
- **Keyboard & LEDs** — keyboard backlight levels, platform LEDs, Fn/Win key
  swap, webcam enable/block.
- **Least privilege** — the UI runs unprivileged; only the exact write you
  confirmed is re-executed through `pkexec` (or `sudo` as fallback). The
  privileged helper refuses any path outside the msi-ec/power-supply/LED trees
  and any malformed value.
- **EC debug interface** — when the driver is loaded with `debug=1`, a gated
  DEBUG tab exposes `ec_get`, `ec_set` and `ec_dump` behind an explicit danger
  confirmation.

## Installation

### Prerequisites

The `msi-ec` kernel module must be loaded.

```sh
# Arch/Manjaro/CachyOS
paru -S msi-ec-dkms-git        # full-featured DKMS driver
# …or the in-tree kernel module (reduced feature set):
sudo modprobe msi-ec
```

See the [upstream install guide](https://github.com/BeardOverflow/msi-ec#installation)
and the [supported device list](https://github.com/BeardOverflow/msi-ec/discussions/277).

### Build

```sh
cargo build --release
sudo cp target/release/msi-ec-tui /usr/local/bin/
```

## Usage

```sh
msi-ec-tui            # run unprivileged; you'll be asked for auth on first write
```

| Key | Action |
| --- | --- |
| `Tab` / `Shift-Tab` | switch tabs |
| `↑/↓` or `j/k` | select an attribute |
| `←/→` or `h/l` | stage a new value (mode, level, threshold) |
| `Enter` / `Space` | commit staged value · flip a toggle |
| `Esc` | discard staged value / close dialogs |
| `r` | refresh the current tab |
| `?` | help overlay |
| `q` | quit |

On the DEBUG tab: `g` read an EC address, `s` write EC memory (dangerous),
`d` load the EC dump, `PgUp/PgDn` scroll it.

### Privilege model

Reads never need root. When a write needs elevation, the UI suspends, runs
`pkexec msi-ec-tui __write <path> <value>` (falling back to `sudo`) and the
re-invoked privileged helper:

- only accepts paths under `/sys/devices/platform/msi-ec/`,
  `/sys/class/power_supply/` and `/sys/class/leds/`,
- rejects path traversal and per-tree value validation
  (integers for batteries/LEDs, hex pairs for the debug interface),
- then performs that single write and exits.

Running the whole TUI as root works too and skips elevation entirely.

### Debug mode (optional)

Load the driver with debug attributes enabled:

```sh
sudo modprobe msi_ec debug=1
```

> **Warning** — writing unknown EC memory addresses may brick your laptop's
> firmware. The EC can be recovered with the reset hole on the bottom of most
> MSI laptops, but consider yourself warned.

## Development

```sh
cargo test                     # unit tests (discovery, validation, key flow)
MSI_EC_SYSROOT=/path/to/fixture cargo run   # run against a fake sysfs tree
```

`MSI_EC_SYSROOT` repoints the UI at another sysfs root — handy for demos on
non-MSI hardware. Privileged writes remain locked to the real `/sys` tree.

## License

MIT. `msi-ec` itself is GPL-2.0; this userspace tool is separate.
