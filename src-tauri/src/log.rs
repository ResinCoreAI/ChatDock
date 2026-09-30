//! Small always-on log (<data>/chatdock.log) so problems seen by the user can be traced afterwards.
//! Only discrete events are logged (open/close/clicks/focus), never page content.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
};

use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 1024 * 1024;

static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();
static ECHO: AtomicBool = AtomicBool::new(false);

pub fn init(dir: &Path, echo: bool) {
    ECHO.store(echo, Ordering::Relaxed);
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("chatdock.log");
    if std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("chatdock.log.old"));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    PATH.set(path).ok();
    FILE.set(Mutex::new(file)).ok();
}

pub fn path() -> Option<&'static PathBuf> {
    PATH.get()
}

/// The Windows account's folder, as paths in the log may spell it (C:\Users\<name>, and the 8.3
/// short form %TEMP% can use): users send the log with bug reports, so it becomes %USERPROFILE%.
fn homes() -> &'static [String] {
    static HOMES: OnceLock<Vec<String>> = OnceLock::new();
    HOMES.get_or_init(|| {
        let mut out: Vec<String> = Vec::new();
        let mut add = |p: &str| {
            let p = p.trim_end_matches('\\');
            if p.len() > 3 && !out.iter().any(|o| o.eq_ignore_ascii_case(p)) {
                out.push(p.to_string());
            }
        };
        if let Some(h) = std::env::var_os("USERPROFILE") {
            add(&h.to_string_lossy());
        }
        for var in ["TEMP", "TMP", "LOCALAPPDATA", "APPDATA"] {
            if let Some(v) = std::env::var_os(var) {
                let v = v.to_string_lossy();
                if let Some(i) = v.to_ascii_lowercase().find("\\appdata") {
                    add(&v[..i]);
                }
            }
        }
        out.sort_by_key(|h| std::cmp::Reverse(h.len()));
        out
    })
}

fn redact(line: &str, homes: &[String]) -> String {
    let mut out = line.to_string();
    for h in homes {
        out = replace_ignoring_case(&out, &h.replace('\\', "\\\\"), "%USERPROFILE%"); // as {:?} writes it
        out = replace_ignoring_case(&out, h, "%USERPROFILE%");
    }
    out
}

fn replace_ignoring_case(text: &str, what: &str, with: &str) -> String {
    let lower = text.to_ascii_lowercase(); // same byte offsets as text
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in lower.match_indices(&what.to_ascii_lowercase()) {
        out.push_str(&text[last..at]);
        out.push_str(with);
        last = at + what.len();
    }
    out.push_str(&text[last..]);
    out
}

pub fn write(msg: &str) {
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}] {}\n",
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
        t.wMilliseconds,
        redact(msg, homes())
    );
    if ECHO.load(Ordering::Relaxed) {
        eprint!("{line}");
    }
    if let Some(lock) = FILE.get() {
        if let Ok(mut f) = lock.lock() {
            if let Some(f) = f.as_mut() {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }
}

/// log!("opened {}", x): one line in chatdock.log
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn the_account_name_stays_out_of_the_log() {
        let homes = [r"C:\Users\GOLFZZ~1".to_string(), r"C:\Users\GolfZzz".to_string()];
        assert_eq!(
            redact(r#"ready {"profile":"C:\\Users\\GolfZzz\\AppData\\Roaming\\ChatDock"}"#, &homes),
            r#"ready {"profile":"%USERPROFILE%\\AppData\\Roaming\\ChatDock"}"#
        );
        assert_eq!(
            redact(r"installer c:\users\golfzzz\AppData\Local\Temp\x.exe", &homes),
            r"installer %USERPROFILE%\AppData\Local\Temp\x.exe"
        );
        assert_eq!(redact(r"temp C:\Users\GOLFZZ~1\AppData\Local\Temp", &homes), r"temp %USERPROFILE%\AppData\Local\Temp");
        assert_eq!(redact("unread discord 0 -> 1", &homes), "unread discord 0 -> 1");
        let thai = [r"C:\Users\สมชาย".to_string()];
        assert_eq!(
            redact(r"exe C:\Users\สมชาย\AppData\Local\ChatDock\ChatDock.exe", &thai),
            r"exe %USERPROFILE%\AppData\Local\ChatDock\ChatDock.exe"
        );
    }
}
