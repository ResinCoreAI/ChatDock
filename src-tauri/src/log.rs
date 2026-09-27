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

pub fn write(msg: &str) {
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}] {}\n",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds, msg
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
