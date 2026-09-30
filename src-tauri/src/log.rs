//! Small always-on log (<data>/chatdock.log) so problems seen by the user can be traced afterwards.
//! Only discrete events are logged (open/close/clicks/focus), never page content.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_BYTES: u64 = 1024 * 1024;
/// Lines one app's pages may cause in a minute (their notifications, messages, permission asks,
/// links); the rest are counted and not written.
const SITE_LINES: u32 = 30;

struct Out {
    file: Option<File>,
    size: u64,
    /// the size at which the log moves (after a move that failed: a while later)
    next_try: u64,
}

static OUT: OnceLock<Mutex<Out>> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();
static ECHO: AtomicBool = AtomicBool::new(false);
/// this is the only ChatDock running: it may move a full log to chatdock.log.old
static ROTATE: AtomicBool = AtomicBool::new(false);

pub fn init(dir: &Path, echo: bool) {
    ECHO.store(echo, Ordering::Relaxed);
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("chatdock.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let size = file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
    PATH.set(path).ok();
    OUT.set(Mutex::new(Out { file, size, next_try: MAX_BYTES })).ok();
}

/// Called once the single-instance lock is held. From then on the log moves to chatdock.log.old
/// whenever it passes 1 MB, also during a long run. A second launch (which only writes its "start"
/// line and hands over to the running one) must never move the running one's log away.
pub fn allow_rotation() {
    ROTATE.store(true, Ordering::Relaxed);
    if let Some(lock) = OUT.get() {
        if let Ok(mut out) = lock.lock() {
            rotate_if_full(&mut out, 0);
        }
    }
}

/// A long session starts a new file too (the last one stays as chatdock.log.old), before a line
/// would take it past 1 MB.
fn rotate_if_full(out: &mut Out, adding: u64) {
    if !ROTATE.load(Ordering::Relaxed) || out.size + adding <= out.next_try {
        return;
    }
    let Some(path) = PATH.get() else { return };
    // Moved while still open (ChatDock's own handle allows that). A scanner or a viewer holding the
    // file may not: then it's tried again 256 KB later, and the log carries on meanwhile.
    if std::fs::rename(path, path.with_extension("log.old")).is_err() {
        out.next_try = out.size + 256 * 1024;
        return;
    }
    if let Ok(new) = OpenOptions::new().create(true).append(true).open(path) {
        out.file = Some(new); // (else it goes on in chatdock.log.old)
        out.size = 0;
    }
    out.next_try = MAX_BYTES;
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
    if let Some(lock) = OUT.get() {
        if let Ok(mut out) = lock.lock() {
            rotate_if_full(&mut out, line.len() as u64);
            if let Some(f) = out.file.as_mut() {
                if f.write_all(line.as_bytes()).is_ok() {
                    out.size += line.len() as u64;
                }
            }
        }
    }
}

/// Whether a line one of an app's pages caused goes in the log: SITE_LINES a minute at most, so a
/// page can't flood it. The first line after a busy minute says how many were left out.
pub fn site_line_ok(app: &str) -> bool {
    static MINUTES: OnceLock<Mutex<HashMap<String, (Instant, u32)>>> = OnceLock::new();
    let Ok(mut minutes) = MINUTES.get_or_init(Default::default).lock() else { return true };
    let (ok, left_out) = site_minute(minutes.entry(app.to_string()).or_insert((Instant::now(), 0)), Instant::now());
    if left_out > 0 {
        write(&format!("{app}: {left_out} more lines its pages caused were left out of the log"));
    }
    ok
}

/// One app's minute (when it began, the lines in it): whether this line goes in, and how many the
/// minute that just ended left out.
fn site_minute(minute: &mut (Instant, u32), now: Instant) -> (bool, u32) {
    let mut left_out = 0;
    if now.duration_since(minute.0) >= Duration::from_secs(60) {
        left_out = minute.1.saturating_sub(SITE_LINES);
        *minute = (now, 0);
    }
    minute.1 += 1;
    (minute.1 <= SITE_LINES, left_out)
}

/// log!("opened {}", x): one line in chatdock.log
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}

/// site_log!(app, "…"): a line one of the app's pages caused (see site_line_ok)
#[macro_export]
macro_rules! site_log {
    ($app:expr, $($arg:tt)*) => {
        if $crate::log::site_line_ok($app) {
            $crate::log::write(&format!($($arg)*))
        }
    };
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{redact, site_minute, SITE_LINES};

    #[test]
    fn a_page_cant_flood_the_log() {
        let start = Instant::now();
        let mut minute = (start, 0);
        for _ in 0..SITE_LINES {
            assert_eq!(site_minute(&mut minute, start), (true, 0));
        }
        assert_eq!(site_minute(&mut minute, start + Duration::from_secs(30)), (false, 0));
        assert_eq!(site_minute(&mut minute, start + Duration::from_secs(59)), (false, 0));
        // the next minute: in again, and the two left out are said
        assert_eq!(site_minute(&mut minute, start + Duration::from_secs(61)), (true, 2));
        assert_eq!(site_minute(&mut minute, start + Duration::from_secs(62)), (true, 0));
    }

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
