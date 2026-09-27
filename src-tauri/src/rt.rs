//! Getting work onto the main thread. Windows, WebView2 and the app state all live there, so
//! everything else (IPC, tray, hotkey, timers, the frame clock) only queues work for it:
//!   post(f)        run f on the main thread soon
//!   after(ms, f)   run f on the main thread in ms milliseconds (cancel with cancel(id))

use std::{
    cmp::Ordering as CmpOrdering,
    collections::{BinaryHeap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

pub fn init(app: AppHandle) {
    APP.set(app).ok();
    START.get_or_init(Instant::now);
    std::thread::Builder::new().name("chatdock-timers".into()).spawn(timer_thread).expect("timer thread");
}

pub fn app() -> &'static AppHandle {
    APP.get().expect("rt::init not called")
}

/// ChatDock's version (tauri.conf.json's, which the installer and the updater use too).
pub fn version() -> String {
    APP.get().map(|a| a.package_info().version.to_string()).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// Milliseconds since start, for animations and time-outs.
pub fn now_ms() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Wall-clock time in ms since 1970 (what settings store, e.g. the do-not-disturb end).
pub fn epoch_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn post(f: impl FnOnce() + Send + 'static) {
    if let Some(app) = APP.get() {
        let _ = app.run_on_main_thread(f);
    }
}

struct Timer {
    due: Instant,
    id: u64,
    f: Box<dyn FnOnce() + Send>,
}

impl PartialEq for Timer {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Timer {}
impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for Timer {
    // BinaryHeap is a max-heap: the earliest due time must compare greatest
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other.due.cmp(&self.due).then_with(|| other.id.cmp(&self.id))
    }
}

#[derive(Default)]
struct Timers {
    heap: BinaryHeap<Timer>,
    pending: HashSet<u64>,
}

static TIMERS: OnceLock<(Mutex<Timers>, Condvar)> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn timers() -> &'static (Mutex<Timers>, Condvar) {
    TIMERS.get_or_init(|| (Mutex::new(Timers::default()), Condvar::new()))
}

/// Run f on the main thread in `ms` milliseconds. Returns an id for cancel().
pub fn after(ms: u64, f: impl FnOnce() + Send + 'static) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (lock, cv) = timers();
    let mut t = lock.lock().unwrap();
    t.pending.insert(id);
    t.heap.push(Timer { due: Instant::now() + Duration::from_millis(ms), id, f: Box::new(f) });
    cv.notify_one();
    id
}

/// Cancel a timer that hasn't fired yet (0 and unknown ids are ignored).
pub fn cancel(id: u64) {
    if id == 0 {
        return;
    }
    let (lock, _) = timers();
    lock.lock().unwrap().pending.remove(&id);
}

fn timer_thread() {
    let (lock, cv) = timers();
    let mut t = lock.lock().unwrap();
    loop {
        let now = Instant::now();
        match t.heap.peek() {
            None => t = cv.wait(t).unwrap(),
            Some(next) if next.due > now => {
                let wait = next.due - now;
                t = cv.wait_timeout(t, wait).unwrap().0;
            }
            Some(_) => {
                let timer = t.heap.pop().unwrap();
                if t.pending.remove(&timer.id) {
                    post(timer.f);
                }
            }
        }
    }
}
