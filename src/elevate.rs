use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Roots the privileged helper is allowed to touch. Anything else is refused.
const ALLOWED_ROOTS: &[&str] = &[
    "/sys/devices/platform/msi-ec/",
    "/sys/class/power_supply/",
    "/sys/class/leds/",
];

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.len() <= 4 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Validate a write request against the privileged-helper allow-list.
///
/// This runs both in the UI (before asking for elevation) and inside the
/// re-executed root helper (`msi-ec-tui __write ...`), so a compromised
/// or tricked UI can never write anywhere arbitrary.
pub fn validate_write(path: &Path, value: &str) -> Result<(), String> {
    let s = path.to_string_lossy();
    if s.split('/').any(|c| c == "..") {
        return Err(format!("path traversal not allowed: {s}"));
    }
    if !ALLOWED_ROOTS.iter().any(|root| s.starts_with(root)) {
        return Err(format!("path not on allow-list: {s}"));
    }
    if value.is_empty() || value.len() > 256 || value.chars().any(|c| (c as u32) < 0x20) {
        return Err(format!("invalid value length/characters: {value:?}"));
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if s.contains("/debug/") {
        // Debug interface: only ec_get (hex address) and ec_set (aa=vv).
        return match name.as_str() {
            "ec_get" if is_hex(value.trim_start_matches("0x").trim_start_matches("0X")) => Ok(()),
            "ec_set" => {
                let Some((addr, val)) = value.split_once('=') else {
                    return Err("ec_set expects 'aa=vv' hex pairs".into());
                };
                if is_hex(addr) && is_hex(val) && addr.len() == 2 && val.len() == 2 {
                    Ok(())
                } else {
                    Err("ec_set expects 'aa=vv' hex pairs".into())
                }
            }
            _ => Err(format!("write not permitted on debug attribute {name}")),
        };
    }

    if s.starts_with("/sys/class/leds/") || s.contains("/power_supply/") {
        return match value.parse::<u8>() {
            Ok(_) => Ok(()),
            Err(_) => Err(format!("expected an integer 0-255, got {value:?}")),
        };
    }

    // Regular msi-ec attributes: on/off, left/right, mode names.
    let ok = value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ' | '+'));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid attribute value: {value:?}"))
    }
}

fn which(prog: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(prog))
            .find(|p| p.is_file())
    })
}

/// Write `value` to `path`, escalating with pkexec/sudo when not root.
pub fn write_escalated(path: &Path, value: &str) -> io::Result<()> {
    validate_write(path, value).map_err(io::Error::other)?;

    // Direct write first: works when running as root or with a udev rule.
    if std::fs::write(path, format!("{value}\n")).is_ok() {
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let exe = exe.to_string_lossy().to_string();

    let mut last_err = String::new();
    for attempt in attempts(&exe) {
        let mut cmd = Command::new(&attempt[0]);
        cmd.args(&attempt[1..])
            .arg("__write")
            .arg(path)
            .arg(value)
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        match cmd.spawn().and_then(|child| child.wait_with_output()) {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                last_err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            }
            Err(e) => last_err = format!("{}: {e}", attempt[0]),
        }
    }
    Err(io::Error::other(if last_err.is_empty() {
        "privileged write failed".to_string()
    } else {
        last_err
    }))
}

/// Escalated read; only used for root-only debug attributes (ec_get/ec_dump).
pub fn read_escalated(path: &Path) -> io::Result<String> {
    let s = path.to_string_lossy();
    if !s.starts_with("/sys/devices/platform/msi-ec/") {
        return Err(io::Error::other(format!("read not permitted: {s}")));
    }
    // Direct read first.
    if let Ok(mut f) = std::fs::File::open(path) {
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_ok() {
            return Ok(buf.trim_end().to_string());
        }
    }
    let exe = std::env::current_exe()?;
    let exe = exe.to_string_lossy().to_string();
    let mut last_err = String::new();
    for attempt in attempts(&exe) {
        let mut cmd = Command::new(&attempt[0]);
        cmd.args(&attempt[1..])
            .arg("__read")
            .arg(path)
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match cmd.spawn().and_then(|child| child.wait_with_output()) {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
                return Ok(text);
            }
            Ok(out) => {
                last_err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            }
            Err(e) => last_err = format!("{}: {e}", attempt[0]),
        }
    }
    Err(io::Error::other(if last_err.is_empty() {
        "privileged read failed".to_string()
    } else {
        last_err
    }))
}

fn attempts(exe: &str) -> Vec<Vec<String>> {
    if which("pkexec").is_some() {
        vec![vec!["pkexec".into(), exe.into()]]
    } else {
        // sudo prompts on the TTY (the UI is suspended while this runs).
        vec![vec!["sudo".into(), "-E".into(), exe.into()]]
    }
}

/// Body of the re-executed privileged helper (`__write` / `__read`).
pub fn helper_main(args: &[String]) -> i32 {
    let mode = args.first().map(String::as_str).unwrap_or("");
    let mut stderr = io::stderr();
    let Some(path) = args.get(1) else {
        let _ = writeln!(
            stderr,
            "usage: msi-ec-tui __write <path> <value> | __read <path>"
        );
        return 2;
    };
    let path = Path::new(path);

    if mode == "__read" {
        let s = path.to_string_lossy();
        if s.split('/').any(|c| c == "..") || !path.starts_with("/sys/devices/platform/msi-ec/") {
            let _ = writeln!(stderr, "refused: read outside msi-ec device tree");
            return 1;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let _ = write!(io::stdout(), "{text}");
                0
            }
            Err(e) => {
                let _ = writeln!(stderr, "read failed: {e}");
                1
            }
        }
    } else {
        let Some(value) = args.get(2) else {
            let _ = writeln!(stderr, "usage: msi-ec-tui __write <path> <value>");
            return 2;
        };
        let value = value.trim_end_matches('\n');
        if let Err(e) = validate_write(path, value) {
            let _ = writeln!(stderr, "refused: {e}");
            return 1;
        }
        match std::fs::write(path, format!("{value}\n")) {
            Ok(()) => 0,
            Err(e) => {
                let _ = writeln!(stderr, "write failed: {e}");
                1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_values() {
        assert!(validate_write(
            Path::new("/sys/devices/platform/msi-ec/shift_mode"),
            "sport"
        )
        .is_ok());
        assert!(validate_write(Path::new("/sys/devices/platform/msi-ec/webcam"), "on").is_ok());
        assert!(validate_write(
            Path::new("/sys/class/power_supply/BAT0/charge_control_start_threshold"),
            "80"
        )
        .is_ok());
        assert!(validate_write(
            Path::new("/sys/class/leds/msiacpi::kbd_backlight/brightness"),
            "3"
        )
        .is_ok());
    }

    #[test]
    fn rejects_paths_outside_allowlist() {
        for p in [
            "/etc/passwd",
            "/sys/firmware/acpi/table",
            "/sys/devices/platform/msi-ec/../msi-ec-webcam/x",
            "/tmp/evil",
        ] {
            assert!(validate_write(Path::new(p), "on").is_err(), "{p} accepted");
        }
    }

    #[test]
    fn rejects_bad_values() {
        // Non-numeric percent
        assert!(validate_write(
            Path::new("/sys/class/power_supply/BAT0/charge_control_start_threshold"),
            "on"
        )
        .is_err());
        // Control chars / shell metacharacters in mode strings
        assert!(validate_write(
            Path::new("/sys/devices/platform/msi-ec/shift_mode"),
            "eco; rm -rf /"
        )
        .is_err());
        assert!(validate_write(
            Path::new("/sys/devices/platform/msi-ec/shift_mode"),
            "eco\noff"
        )
        .is_err());
    }

    #[test]
    fn debug_writes_are_strict() {
        let get = Path::new("/sys/devices/platform/msi-ec/debug/ec_get");
        let set = Path::new("/sys/devices/platform/msi-ec/debug/ec_set");
        assert!(validate_write(get, "f5").is_ok());
        assert!(validate_write(get, "0x9c").is_ok());
        assert!(validate_write(get, "zz").is_err());
        assert!(validate_write(set, "f5=01").is_ok());
        assert!(validate_write(set, "f5=1").is_err());
        assert!(validate_write(set, "f5 01").is_err());
        let dump = Path::new("/sys/devices/platform/msi-ec/debug/ec_dump");
        assert!(validate_write(dump, "f5=01").is_err());
    }
}
