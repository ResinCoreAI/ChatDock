//! Small always-on log (<data>/chatdock.log) so problems seen by the user can be traced afterwards.
//! Only discrete events are logged (open/close/clicks/focus), never page content.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 1024 * 1024;

static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();
static ECHO: AtomicBool = AtomicBool::new(false);
/// bytes in chatdock.log now
static SIZE: AtomicU64 = AtomicU64::new(0);
/// this is the only ChatDock running: it may move a full log to chatdock.log.old
static ROTATE: AtomicBool = AtomicBool::new(false);

pub fn init(dir: &Path, echo: bool) {
    ECHO.store(echo, Ordering::Relaxed);
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("chatdock.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    SIZE.store(file.as_ref().and_then(|f| f.metadata().ok()).map(|m| m.len()).unwrap_or(0), Ordering::Relaxed);
    PATH.set(path).ok();
    FILE.set(Mutex::new(file)).ok();
}

/// Called once the single-instance lock is held. From then on the log moves to chatdock.log.old
/// whenever it passes 1 MB, also during a long run. A second launch (which only writes its "start"
/// line and hands over to the running one) must never move the running one's log away.
pub fn allow_rotation() {
    ROTATE.store(true, Ordering::Relaxed);
    if let Some(lock) = FILE.get() {
        if let Ok(mut f) = lock.lock() {
            rotate_if_full(&mut f);
        }
    }
}

fn rotate_if_full(file: &mut Option<File>) {
    if !ROTATE.load(Ordering::Relaxed) || SIZE.load(Ordering::Relaxed) <= MAX_BYTES {
        return;
    }
    let Some(path) = PATH.get() else { return };
    *file = None; // closed before it moves
    let _ = std::fs::rename(path, path.with_extension("log.old"));
    *file = OpenOptions::new().create(true).append(true).open(path).ok();
    SIZE.store(file.as_ref().and_then(|f| f.metadata().ok()).map(|m| m.len()).unwrap_or(0), Ordering::Relaxed);
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
            if let Some(file) = f.as_mut() {
                if file.write_all(line.as_bytes()).is_ok() {
                    SIZE.fetch_add(line.len() as u64, Ordering::Relaxed);
                }
            }
            rotate_if_full(&mut f);
        }
    }
}

/// log!("opened {}", x): one line in chatdock.log
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
