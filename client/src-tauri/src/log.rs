//! Append-only log in the config folder, so problems in the tray app can be diagnosed.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config;

const MAX_BYTES: u64 = 1_000_000;

pub fn path() -> PathBuf {
    config::config_dir().join("ems-solver.log")
}

/// UTC timestamp like 2026-10-18T20:31:05Z, without pulling in a date crate.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn line(message: impl AsRef<str>) {
    let entry = format!("{} {}\n", timestamp(), message.as_ref());
    eprint!("{entry}");
    let path = path();
    let _ = fs::create_dir_all(config::config_dir());
    if fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(entry.as_bytes());
    }
}

/// Panics in background tasks are otherwise silent.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        line(format!("panic: {info}"));
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_shape() {
        let ts = super::timestamp();
        assert_eq!(ts.len(), 20);
        assert!(ts.starts_with("20") && ts.ends_with('Z'));
    }
}
