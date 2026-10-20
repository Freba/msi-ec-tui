use std::fmt;
use std::path::PathBuf;

/// Every attribute of the msi-ec driver that the TUI knows about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AttrId {
    Webcam,
    WebcamBlock,
    FnKey,
    WinKey,
    CoolerBoost,
    SuperBattery,
    ShiftMode,
    FanMode,
    CpuTemp,
    CpuFan,
    GpuTemp,
    GpuFan,
    FwVersion,
    FwReleaseDate,
    BatteryStart,
    BatteryEnd,
    KbdBacklight,
    Led(PathBuf),
    Raw(PathBuf),
    EcFwVersion,
    EcGet,
    EcSet,
}

impl fmt::Display for AttrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AttrId::Webcam => "webcam",
            AttrId::WebcamBlock => "webcam_block",
            AttrId::FnKey => "fn_key",
            AttrId::WinKey => "win_key",
            AttrId::CoolerBoost => "cooler_boost",
            AttrId::SuperBattery => "super_battery",
            AttrId::ShiftMode => "shift_mode",
            AttrId::FanMode => "fan_mode",
            AttrId::CpuTemp => "cpu/realtime_temperature",
            AttrId::CpuFan => "cpu/realtime_fan_speed",
            AttrId::GpuTemp => "gpu/realtime_temperature",
            AttrId::GpuFan => "gpu/realtime_fan_speed",
            AttrId::FwVersion => "fw_version",
            AttrId::FwReleaseDate => "fw_release_date",
            AttrId::BatteryStart => "charge_control_start_threshold",
            AttrId::BatteryEnd => "charge_control_end_threshold",
            AttrId::KbdBacklight => "kbd_backlight",
            AttrId::Led(p) => return write!(f, "led:{}", p.display()),
            AttrId::Raw(p) => return write!(f, "raw:{}", p.display()),
            AttrId::EcFwVersion => "debug/fw_version",
            AttrId::EcGet => "debug/ec_get",
            AttrId::EcSet => "debug/ec_set",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// on/off boolean
    Toggle,
    /// left/right keyboard key swap
    SwitchKey,
    /// value picked from a companion `available_*` file
    Enum,
    /// writable 0..max integer (battery thresholds, leds)
    Percent { max: u8 },
    /// live numeric sensor value
    Sensor { unit: &'static str, max: f64 },
    /// read-only text
    ReadText,
    /// debug: write an address, read back the EC byte
    EcGet,
    /// debug: write "aa=vv" straight into EC memory
    EcSet,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Text(String),
    Pct(u8),
    Num(f64),
}

impl Value {
    /// String form used when writing the value back to sysfs.
    pub fn as_wire(&self) -> String {
        match self {
            Value::Bool(true) => "on".into(),
            Value::Bool(false) => "off".into(),
            Value::Text(s) => s.clone(),
            Value::Pct(p) => p.to_string(),
            Value::Num(n) => n.trunc().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Value(Value),
    /// Attribute not exported by this driver/firmware.
    Unavailable,
    /// File exists but could not be read.
    Error(String),
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub id: AttrId,
    pub label: String,
    pub path: PathBuf,
    pub kind: Kind,
    /// Writable *per driver spec* (does not reflect file permissions).
    pub writable: bool,
    pub state: State,
    /// Options for Kind::Enum, sourced from the companion available_* file.
    pub options: Vec<String>,
    /// Value staged with the arrow keys but not yet committed.
    pub staged: Option<Value>,
}

impl Attr {
    pub fn new(
        id: AttrId,
        label: impl Into<String>,
        path: impl Into<PathBuf>,
        kind: Kind,
        writable: bool,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            path: path.into(),
            kind,
            writable,
            state: State::Unavailable,
            options: Vec::new(),
            staged: None,
        }
    }

    pub fn current(&self) -> Option<&Value> {
        match &self.state {
            State::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Parse a raw sysfs string into a typed value for this attribute.
    pub fn parse(&self, raw: &str) -> Value {
        let raw = raw.trim();
        match &self.kind {
            Kind::Toggle => Value::Bool(raw == "on" || raw == "1"),
            Kind::SwitchKey | Kind::Enum | Kind::ReadText | Kind::EcGet => {
                Value::Text(raw.to_string())
            }
            Kind::Percent { .. } | Kind::EcSet => {
                let s = raw.strip_suffix('%').unwrap_or(raw).trim();
                if let Ok(p) = s.parse::<u8>() {
                    Value::Pct(p)
                } else if let Ok(f) = s.parse::<f64>() {
                    Value::Num(f)
                } else {
                    Value::Text(s.to_string())
                }
            }
            Kind::Sensor { .. } => {
                if let Ok(f) = raw.parse::<f64>() {
                    Value::Num(f)
                } else {
                    // tolerate suffixes like "62 C" or "80 %"
                    let trimmed = raw
                        .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.')
                        .trim_end();
                    trimmed
                        .parse::<f64>()
                        .map(Value::Num)
                        .unwrap_or_else(|_| Value::Text(raw.to_string()))
                }
            }
        }
    }
}
