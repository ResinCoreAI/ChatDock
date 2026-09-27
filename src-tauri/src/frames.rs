//! Frame clock for windows ChatDock moves itself (the panel sliding in, the edge tab following the
//! cursor, the hold line). A moving window only looks smooth when it moves once per screen refresh;
//! plain timers fire every 15.6 ms on Windows (64 times a second), so on a 144-300 Hz screen a window
//! moved on a timer visibly jumps. A thread waits for the screen's vertical blank
//! (D3DKMTWaitForVerticalBlankEvent, the same wait Chromium uses) and hands each frame to the main
//! thread. Without it: a high-resolution timer at the refresh interval. Nothing runs while nothing
//! animates, and a stalled screen (turned off) can't freeze an animation: a watchdog steps in.

use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex, OnceLock,
    },
    time::Duration,
};

use windows::{
    core::{HSTRING, PCWSTR},
    Wdk::Graphics::Direct3D::{
        D3DKMTCloseAdapter, D3DKMTOpenAdapterFromHdc, D3DKMTWaitForVerticalBlankEvent, D3DKMT_CLOSEADAPTER, D3DKMT_OPENADAPTERFROMHDC,
        D3DKMT_WAITFORVERTICALBLANKEVENT,
    },
    Win32::{
        Foundation::CloseHandle,
        Graphics::Gdi::{CreateDCW, DeleteDC},
        System::Threading::{
            CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE,
            TIMER_ALL_ACCESS,
        },
    },
};

use crate::{log, rt};

struct Shared {
    wanted: bool,
    adapter: Option<(u32, u32)>, // (hAdapter, VidPnSourceId)
    hz: u32,
    device: String,
}

static SHARED: OnceLock<(Mutex<Shared>, Condvar)> = OnceLock::new();
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

type FrameFn = Box<dyn FnOnce(f64)>;

thread_local! {
    static QUEUE: RefCell<Vec<FrameFn>> = const { RefCell::new(Vec::new()) };
    static LAST_TICK: RefCell<f64> = const { RefCell::new(0.0) };
    static WATCHDOG: RefCell<u64> = const { RefCell::new(0) };
}

fn shared() -> &'static (Mutex<Shared>, Condvar) {
    SHARED.get_or_init(|| (Mutex::new(Shared { wanted: false, adapter: None, hz: 60, device: String::new() }), Condvar::new()))
}

pub fn init() {
    std::thread::Builder::new().name("chatdock-vblank".into()).spawn(vblank_thread).expect("vblank thread");
}

/// The screen the dock is on: its refresh rate, and the adapter whose vertical blank times frames.
pub fn set_display(device: &str, hz: u32) {
    let (lock, _) = shared();
    let mut s = lock.lock().unwrap();
    s.hz = hz.max(24);
    if s.device == device && s.adapter.is_some() {
        return;
    }
    if let Some((old, _)) = s.adapter.take() {
        unsafe {
            let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: old });
        }
    }
    s.device = device.to_string();
    unsafe {
        let name = HSTRING::from(device);
        let hdc = CreateDCW(&name, &name, PCWSTR::null(), None);
        if hdc.is_invalid() {
            log!("frame clock: no device context for {device}");
            return;
        }
        let mut open = D3DKMT_OPENADAPTERFROMHDC { hDc: hdc, ..Default::default() };
        let status = D3DKMTOpenAdapterFromHdc(&mut open);
        let _ = DeleteDC(hdc);
        if status.is_ok() {
            s.adapter = Some((open.hAdapter, open.VidPnSourceId));
        } else {
            log!("frame clock: no vertical blank for {device} ({:#x}), using a timer", status.0);
        }
    }
}

pub fn info() -> (u32, &'static str) {
    let (lock, _) = shared();
    let s = lock.lock().unwrap();
    (s.hz, if s.adapter.is_some() { "vblank" } else { "timer" })
}

/// Like requestAnimationFrame: cb(now_ms) runs once, on the main thread, at the next screen refresh.
pub fn request(cb: impl FnOnce(f64) + 'static) {
    let was_empty = QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        q.push(Box::new(cb));
        q.len() == 1
    });
    if was_empty {
        let (lock, cv) = shared();
        lock.lock().unwrap().wanted = true;
        cv.notify_one();
        arm_watchdog();
    }
}

fn arm_watchdog() {
    WATCHDOG.with(|w| {
        rt::cancel(*w.borrow());
        *w.borrow_mut() = rt::after(80, || {
            WATCHDOG.with(|w| *w.borrow_mut() = 0);
            let stale = rt::now_ms() - LAST_TICK.with(|t| *t.borrow()) > 60.0;
            let waiting = QUEUE.with(|q| !q.borrow().is_empty());
            if waiting && stale {
                tick(); // the vertical blank stopped coming (screen off): keep animations going
            }
            if QUEUE.with(|q| !q.borrow().is_empty()) {
                arm_watchdog();
            }
        });
    });
}

fn tick() {
    IN_FLIGHT.store(false, Ordering::Release);
    let now = rt::now_ms();
    LAST_TICK.with(|t| *t.borrow_mut() = now);
    let due: Vec<FrameFn> = QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for cb in due {
        cb(now);
    }
    if QUEUE.with(|q| q.borrow().is_empty()) {
        shared().0.lock().unwrap().wanted = false;
    }
}

fn precise_sleep(ms: f64) {
    unsafe {
        if let Ok(timer) = CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) {
            let due = -((ms * 10_000.0) as i64); // relative, in 100 ns units
            if SetWaitableTimer(timer, &due, 0, None, None, false).is_ok() {
                WaitForSingleObject(timer, INFINITE);
            }
            let _ = CloseHandle(timer);
        } else {
            std::thread::sleep(Duration::from_secs_f64(ms / 1000.0));
        }
    }
}

fn vblank_thread() {
    let (lock, cv) = shared();
    let mut failures = 0;
    loop {
        let (adapter, hz) = {
            let mut s = lock.lock().unwrap();
            while !s.wanted {
                s = cv.wait(s).unwrap();
            }
            (s.adapter, s.hz)
        };
        let waited = match adapter {
            Some((h_adapter, source)) if failures < 3 => {
                let wait = D3DKMT_WAITFORVERTICALBLANKEVENT { hAdapter: h_adapter, hDevice: 0, VidPnSourceId: source };
                let ok = unsafe { D3DKMTWaitForVerticalBlankEvent(&wait) }.is_ok();
                failures = if ok { 0 } else { failures + 1 };
                ok
            }
            _ => false,
        };
        if !waited {
            precise_sleep(1000.0 / hz as f64);
        }
        if !IN_FLIGHT.swap(true, Ordering::AcqRel) {
            rt::post(tick);
        }
    }
}
