pub mod attr;

use crate::backend::EcBackend;
use crate::model::attr::{Attr, AttrId, Kind, State};
use std::path::{Path, PathBuf};

pub const MSI_EC_ATTRS: &[AttrId] = &[
    AttrId::Webcam,
    AttrId::WebcamBlock,
    AttrId::FnKey,
    AttrId::WinKey,
    AttrId::CoolerBoost,
    AttrId::SuperBattery,
    AttrId::ShiftMode,
    AttrId::FanMode,
    AttrId::CpuTemp,
    AttrId::CpuFan,
    AttrId::GpuTemp,
    AttrId::GpuFan,
    AttrId::FwVersion,
    AttrId::FwReleaseDate,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabId {
    Performance,
    Thermals,
    Battery,
    InputLed,
    Info,
    Debug,
}

impl TabId {
    pub fn title(self) -> &'static str {
        match self {
            TabId::Performance => "Performance",
            TabId::Thermals => "Thermals",
            TabId::Battery => "Battery",
            TabId::InputLed => "Input & LED",
            TabId::Info => "Info",
            TabId::Debug => "DEBUG",
        }
    }
}

#[derive(Debug)]
pub struct Group {
    pub tab: TabId,
    pub attrs: Vec<Attr>,
}

#[derive(Debug)]
pub struct Discovery {
    /// sysfs root, normally "/sys".
    pub sysroot: PathBuf,
    pub device_present: bool,
    pub debug_present: bool,
    pub groups: Vec<Group>,
}

impl Discovery {
    pub fn msi_ec_dir(&self) -> PathBuf {
        self.sysroot.join("devices/platform/msi-ec")
    }

    pub fn group(&self, tab: TabId) -> Option<&Group> {
        self.groups.iter().find(|g| g.tab == tab)
    }

    pub fn group_mut(&mut self, tab: TabId) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.tab == tab)
    }

    pub fn find(&self, id: &AttrId) -> Option<&Attr> {
        self.groups
            .iter()
            .flat_map(|g| &g.attrs)
            .find(|a| &a.id == id)
    }

    pub fn find_mut(&mut self, id: &AttrId) -> Option<&mut Attr> {
        self.groups
            .iter_mut()
            .flat_map(|g| &mut g.attrs)
            .find(|a| &a.id == id)
    }
}

fn read_options<B: EcBackend>(b: &B, path: &Path) -> Vec<String> {
    b.read(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn load<B: EcBackend>(b: &B, attr: &mut Attr) {
    if !b.exists(&attr.path) {
        attr.state = State::Unavailable;
        return;
    }
    match b.read(&attr.path) {
        Ok(raw) => attr.state = State::Value(attr.parse(&raw)),
        Err(e) => attr.state = State::Error(e.to_string()),
    }
}

pub fn discover<B: EcBackend>(b: &B, sysroot: impl Into<PathBuf>) -> Discovery {
    let sysroot: PathBuf = sysroot.into();
    let ec = sysroot.join("devices/platform/msi-ec");
    let device_present = b.exists(&ec);
    let debug_dir = ec.join("debug");
    let debug_present = device_present && b.exists(&debug_dir);

    let mut performance = vec![{
        let mut a = Attr::new(
            AttrId::ShiftMode,
            "Shift mode",
            ec.join("shift_mode"),
            Kind::Enum,
            true,
        );
        a.options = read_options(b, &ec.join("available_shift_modes"));
        a
    }];
    performance.push(Attr::new(
        AttrId::CoolerBoost,
        "Cooler boost",
        ec.join("cooler_boost"),
        Kind::Toggle,
        true,
    ));

    let mut thermals = vec![{
        let mut a = Attr::new(
            AttrId::FanMode,
            "Fan mode",
            ec.join("fan_mode"),
            Kind::Enum,
            true,
        );
        a.options = read_options(b, &ec.join("available_fan_modes"));
        a
    }];
    thermals.extend([
        Attr::new(
            AttrId::CpuTemp,
            "CPU temperature",
            ec.join("cpu/realtime_temperature"),
            Kind::Sensor {
                unit: "°C",
                max: 110.0,
            },
            false,
        ),
        Attr::new(
            AttrId::CpuFan,
            "CPU fan",
            ec.join("cpu/realtime_fan_speed"),
            Kind::Sensor {
                unit: "%",
                max: 150.0,
            },
            false,
        ),
        Attr::new(
            AttrId::GpuTemp,
            "GPU temperature",
            ec.join("gpu/realtime_temperature"),
            Kind::Sensor {
                unit: "°C",
                max: 110.0,
            },
            false,
        ),
        Attr::new(
            AttrId::GpuFan,
            "GPU fan",
            ec.join("gpu/realtime_fan_speed"),
            Kind::Sensor {
                unit: "%",
                max: 150.0,
            },
            false,
        ),
    ]);

    // Battery: find a power_supply that exposes charge control thresholds.
    let mut battery: Vec<Attr> = Vec::new();
    let supplies = sysroot.join("class/power_supply");
    let mut chosen: Option<PathBuf> = None;
    for supply in b.read_dir(&supplies) {
        if b.exists(&supply.join("charge_control_start_threshold"))
            || b.exists(&supply.join("charge_control_end_threshold"))
        {
            chosen = Some(supply);
            break;
        }
    }
    if let Some(supply) = chosen {
        battery.push(Attr::new(
            AttrId::BatteryStart,
            "Charge start threshold",
            supply.join("charge_control_start_threshold"),
            Kind::Percent { max: 100 },
            true,
        ));
        battery.push(Attr::new(
            AttrId::BatteryEnd,
            "Charge stop threshold",
            supply.join("charge_control_end_threshold"),
            Kind::Percent { max: 100 },
            true,
        ));
    }
    battery.push(Attr::new(
        AttrId::SuperBattery,
        "Super battery",
        ec.join("super_battery"),
        Kind::Toggle,
        true,
    ));

    // Input, webcam and LEDs.
    let mut input_led: Vec<Attr> = vec![
        Attr::new(
            AttrId::FnKey,
            "Fn key position",
            ec.join("fn_key"),
            Kind::SwitchKey,
            true,
        ),
        Attr::new(
            AttrId::WinKey,
            "Windows key position",
            ec.join("win_key"),
            Kind::SwitchKey,
            true,
        ),
        Attr::new(
            AttrId::Webcam,
            "Webcam",
            ec.join("webcam"),
            Kind::Toggle,
            true,
        ),
        Attr::new(
            AttrId::WebcamBlock,
            "Webcam block",
            ec.join("webcam_block"),
            Kind::Toggle,
            true,
        ),
    ];
    for led in b.read_dir(&sysroot.join("class/leds")) {
        let name = led
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let max = b
            .read(&led.join("max_brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(1)
            .max(1);
        if led.to_string_lossy().contains("kbd_backlight") {
            input_led.push(Attr::new(
                AttrId::KbdBacklight,
                "Keyboard backlight",
                led.join("brightness"),
                Kind::Percent { max },
                true,
            ));
        } else if b.exists(&led.join("brightness")) {
            input_led.push(Attr::new(
                AttrId::Led(led.clone()),
                name,
                led.join("brightness"),
                Kind::Percent { max },
                true,
            ));
        }
    }

    // Info + raw fallback for anything we don't map explicitly.
    let mut info = vec![
        Attr::new(
            AttrId::FwVersion,
            "Firmware version",
            ec.join("fw_version"),
            Kind::ReadText,
            false,
        ),
        Attr::new(
            AttrId::FwReleaseDate,
            "Firmware release date",
            ec.join("fw_release_date"),
            Kind::ReadText,
            false,
        ),
    ];
    for entry in b.read_dir(&ec) {
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == "debug"
            || name == "available_shift_modes"
            || name == "available_fan_modes"
            || entry.is_dir()
        {
            continue;
        }
        let mapped = MSI_EC_ATTRS
            .iter()
            .any(|id| id.to_string() == name || id.to_string().starts_with(&format!("{name}/")));
        if !mapped {
            info.push(Attr::new(
                AttrId::Raw(entry.clone()),
                name,
                entry,
                Kind::ReadText,
                false,
            ));
        }
    }

    let mut groups = vec![
        Group {
            tab: TabId::Performance,
            attrs: performance,
        },
        Group {
            tab: TabId::Thermals,
            attrs: thermals,
        },
        Group {
            tab: TabId::Battery,
            attrs: battery,
        },
        Group {
            tab: TabId::InputLed,
            attrs: input_led,
        },
        Group {
            tab: TabId::Info,
            attrs: info,
        },
    ];

    if debug_present {
        let dbg = vec![
            Attr::new(
                AttrId::EcFwVersion,
                "EC firmware",
                debug_dir.join("fw_version"),
                Kind::ReadText,
                false,
            ),
            Attr::new(
                AttrId::EcGet,
                "ec_get",
                debug_dir.join("ec_get"),
                Kind::EcGet,
                true,
            ),
            Attr::new(
                AttrId::EcSet,
                "ec_set",
                debug_dir.join("ec_set"),
                Kind::EcSet,
                true,
            ),
        ];
        groups.push(Group {
            tab: TabId::Debug,
            attrs: dbg,
        });
    }

    let mut disc = Discovery {
        sysroot,
        device_present,
        debug_present,
        groups,
    };
    disc.refresh_all(b);
    disc
}

impl Discovery {
    /// (Re-)read every attribute that is not special-cased.
    pub fn refresh_all<B: EcBackend>(&mut self, b: &B) {
        for group in &mut self.groups {
            for attr in &mut group.attrs {
                if matches!(attr.kind, Kind::EcGet | Kind::EcSet) {
                    continue;
                }
                load(b, attr);
            }
        }
    }

    pub fn refresh_tab<B: EcBackend>(&mut self, b: &B, tab: TabId) {
        if tab == TabId::Debug {
            return;
        }
        if let Some(group) = self.group_mut(tab) {
            for attr in &mut group.attrs {
                load(b, attr);
            }
        }
    }

    /// Fast path for the periodic tick: only live sensors.
    pub fn refresh_sensors<B: EcBackend>(&mut self, b: &B) {
        let sensors = [
            AttrId::CpuTemp,
            AttrId::CpuFan,
            AttrId::GpuTemp,
            AttrId::GpuFan,
        ];
        for id in sensors {
            if let Some(attr) = self.find_mut(&id) {
                load(b, attr);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::model::attr::Value;

    fn fixture() -> FakeBackend {
        let mut f = FakeBackend::new();
        let ec = "/sys/devices/platform/msi-ec";
        f.add(format!("{ec}/shift_mode"), "comfort\n")
            .add(
                format!("{ec}/available_shift_modes"),
                "eco\ncomfort\nsport\nturbo",
            )
            .add(format!("{ec}/fan_mode"), "auto\n")
            .add(
                format!("{ec}/available_fan_modes"),
                "auto\nsilent\nbasic\nadvanced",
            )
            .add(format!("{ec}/cooler_boost"), "off")
            .add(format!("{ec}/cpu/realtime_temperature"), "54")
            .add(format!("{ec}/cpu/realtime_fan_speed"), "35")
            .add(format!("{ec}/fw_version"), "E16U5IMS.107")
            .add("/sys/class/power_supply/BAT0/type", "Battery")
            .add(
                "/sys/class/power_supply/BAT0/charge_control_start_threshold",
                "75",
            )
            .add(
                "/sys/class/power_supply/BAT0/charge_control_end_threshold",
                "80\n",
            )
            .add("/sys/class/leds/msiacpi::kbd_backlight/max_brightness", "3")
            .add("/sys/class/leds/msiacpi::kbd_backlight/brightness", "2");
        f
    }

    #[test]
    fn discovers_known_attrs() {
        let disc = discover(&fixture(), "/sys");
        assert!(disc.device_present);
        assert!(!disc.debug_present);

        let shift = disc.find(&AttrId::ShiftMode).unwrap();
        assert_eq!(shift.current(), Some(&Value::Text("comfort".into())));
        assert_eq!(shift.options, ["eco", "comfort", "sport", "turbo"]);

        let start = disc.find(&AttrId::BatteryStart).unwrap();
        assert_eq!(start.current(), Some(&Value::Pct(75)));
        assert_eq!(
            start.path,
            PathBuf::from("/sys/class/power_supply/BAT0/charge_control_start_threshold")
        );

        let kbd = disc.find(&AttrId::KbdBacklight).unwrap();
        assert_eq!(kbd.current(), Some(&Value::Pct(2)));
        assert_eq!(kbd.kind, Kind::Percent { max: 3 });

        let cpu = disc.find(&AttrId::CpuTemp).unwrap();
        assert_eq!(cpu.current(), Some(&Value::Num(54.0)));
    }

    #[test]
    fn missing_attrs_are_unavailable() {
        let disc = discover(&fixture(), "/sys");
        // webcam / super_battery are absent in the fixture
        assert_eq!(
            disc.find(&AttrId::Webcam).unwrap().state,
            State::Unavailable
        );
        assert_eq!(
            disc.find(&AttrId::GpuTemp).unwrap().state,
            State::Unavailable
        );
    }

    #[test]
    fn unmapped_files_show_up_as_raw() {
        let mut f = fixture();
        f.add("/sys/devices/platform/msi-ec/some_new_attr", "42");
        let disc = discover(&f, "/sys");
        let raw = disc.group(TabId::Info).unwrap().attrs.iter().find(|a| {
            a.id == AttrId::Raw(PathBuf::from("/sys/devices/platform/msi-ec/some_new_attr"))
        });
        assert!(
            raw.is_some(),
            "unmapped attribute should surface as raw entry"
        );
    }

    #[test]
    fn debug_tab_only_when_present() {
        let disc = discover(&fixture(), "/sys");
        assert!(disc.group(TabId::Debug).is_none());

        let mut f = fixture();
        f.add("/sys/devices/platform/msi-ec/debug/ec_get", "00");
        f.add("/sys/devices/platform/msi-ec/debug/ec_set", "");
        let disc = discover(&f, "/sys");
        assert!(disc.debug_present);
        assert!(disc.group(TabId::Debug).is_some());
    }

    #[test]
    fn no_device_yields_empty_discovery() {
        let f = FakeBackend::new();
        let disc = discover(&f, "/sys");
        assert!(!disc.device_present);
        for group in &disc.groups {
            for attr in &group.attrs {
                assert_eq!(attr.state, State::Unavailable, "{}", attr.id);
            }
        }
    }
}
