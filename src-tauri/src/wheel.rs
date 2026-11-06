//! The mouse wheel on the dock edge: turning it there means scrolling the page under the pointer (a
//! maximized browser's scrollbar sits right at the screen edge), not asking for the chat. A small
//! thread gets a copy of the mouse's raw input only while the edge is being held or the tab is out,
//! and notes when the wheel last turned. Raw input is a copy: it never holds up the mouse for
//! anything else, a game included.
//!
//! (Tauri's window layer registers the mouse for raw input for its device events, which ChatDock
//! doesn't use; a process has one registration per device kind, so this takes it over meanwhile.)
//!
//! Discord push-to-talk (1.8) reads its key here the same way: a copy of the keyboard (or of the
//! mouse's side buttons) while a key is chosen. The game gets every key as before.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, AtomicU32, Ordering};

use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Input::{
                GetRawInputData, KeyboardAndMouse::GetAsyncKeyState, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
                RAWINPUTHEADER, RIDEV_INPUTSINK, RIDEV_REMOVE, RID_INPUT, RIM_TYPEMOUSE,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW, RegisterClassW, HWND_MESSAGE, MSG,
                RI_MOUSE_HWHEEL, RI_MOUSE_WHEEL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WNDCLASSW,
            },
        },
    },
};

use crate::{log, rt};

const WM_LISTEN: u32 = WM_APP + 7;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static LISTENING: AtomicBool = AtomicBool::new(false);
static LAST: AtomicI64 = AtomicI64::new(0);
/// the push-to-talk key (key_code), 0 = none; whether it is held right now
static PTT: AtomicU32 = AtomicU32::new(0);
static PTT_DOWN: AtomicBool = AtomicBool::new(false);
/// a push-to-talk key (not a side button) is read every 15 ms while this is on (in a Discord call);
/// each start bumps the generation, so an older loop stops
static POLLING: AtomicBool = AtomicBool::new(false);
static POLL_GEN: AtomicU32 = AtomicU32::new(0);
/// self-test only: the push-to-talk key counts as held
static TEST_HELD: AtomicBool = AtomicBool::new(false);

/// Self-test: the push-to-talk key held or not (no real key in a test run).
pub fn test_hold(held: bool) {
    TEST_HELD.store(held, Ordering::Relaxed);
}

/// The mouse's side buttons, as push-to-talk "keys" (above any virtual-key code)
pub const MOUSE4: u32 = 0x105;
pub const MOUSE5: u32 = 0x106;

/// A push-to-talk key's name (the browser's KeyboardEvent.code, or Mouse4 / Mouse5) -> its code.
pub fn key_code(name: &str) -> Option<u32> {
    let one = |p: &str, lo: char, hi: char| {
        name.strip_prefix(p).filter(|r| r.len() == 1).and_then(|r| r.chars().next()).filter(|c| (lo..=hi).contains(c))
    };
    if let Some(c) = one("Key", 'A', 'Z').or_else(|| one("Digit", '0', '9')) {
        return Some(c as u32);
    }
    if let Some(n) = name.strip_prefix('F').and_then(|r| r.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(0x6F + n);
    }
    match name {
        "Mouse4" => Some(MOUSE4),
        "Mouse5" => Some(MOUSE5),
        "Backquote" => Some(0xC0),
        "CapsLock" => Some(0x14),
        "Insert" => Some(0x2D),
        "Home" => Some(0x24),
        "PageUp" => Some(0x21),
        "PageDown" => Some(0x22),
        _ => None,
    }
}

/// Keys a game moves with all the time are refused (talking whenever you walk).
pub fn key_ok(name: &str) -> bool {
    key_code(name).is_some() && !["KeyW", "KeyA", "KeyS", "KeyD"].contains(&name)
}

/// The push-to-talk key (0 = none): its raw input is copied while it is set.
pub fn set_ptt(code: u32) {
    if PTT.swap(code, Ordering::Relaxed) == code {
        return;
    }
    PTT_DOWN.store(false, Ordering::Relaxed);
    post_listen();
}

/// The push-to-talk key is held right now (for when a key-up went missing).
pub fn ptt_held() -> bool {
    if TEST_HELD.load(Ordering::Relaxed) {
        return true;
    }
    let vk = match PTT.load(Ordering::Relaxed) {
        0 => return false,
        MOUSE4 => 0x05, // VK_XBUTTON1
        MOUSE5 => 0x06,
        k => k as i32,
    };
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

fn post_listen() {
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_LISTEN, WPARAM(0), LPARAM(0));
        }
    }
}

/// A push-to-talk key that is a keyboard key (read by polling; the side buttons come as raw input).
pub fn is_key(code: u32) -> bool {
    code != 0 && code != MOUSE4 && code != MOUSE5
}

/// One poll of the key: Some(down) when it changed since the last one.
fn poll_edge(was: bool, held: bool) -> Option<bool> {
    (was != held).then_some(held)
}

/// Reads the push-to-talk key every 15 ms while `on` (a keyboard key, in a Discord call); no
/// keyboard input is copied from other programs. Stopping while it is held lets it go (muted).
pub fn poll(on: bool) {
    if !on {
        if POLLING.swap(false, Ordering::Relaxed) {
            POLL_GEN.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    if POLLING.swap(true, Ordering::Relaxed) {
        return;
    }
    let gen = POLL_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let started = std::thread::Builder::new().name("chatdock-ptt".into()).spawn(move || {
        while POLLING.load(Ordering::Relaxed) && POLL_GEN.load(Ordering::Relaxed) == gen {
            if let Some(down) = poll_edge(PTT_DOWN.load(Ordering::Relaxed), ptt_held()) {
                ptt_change(down);
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        ptt_change(false);
    });
    if let Err(err) = started {
        POLLING.store(false, Ordering::Relaxed);
        log!("push-to-talk: no thread: {err}");
    }
}

/// The push-to-talk key is being polled now.
pub fn polling() -> bool {
    POLLING.load(Ordering::Relaxed)
}

/// One raw mouse event against the push-to-talk side button: Some(down) when it is that button
/// going down or up (its button flags: side button 4 down 0x40 / up 0x80, 5 0x100 / 0x200).
fn ptt_edge(buttons: u32, ptt: u32) -> Option<bool> {
    let (down, up) = match ptt {
        MOUSE4 => (0x40, 0x80),
        MOUSE5 => (0x100, 0x200),
        _ => return None,
    };
    if buttons & down != 0 {
        Some(true)
    } else if buttons & up != 0 {
        Some(false)
    } else {
        None
    }
}

/// The key went down or up (repeats while held don't count).
fn ptt_change(down: bool) {
    if PTT_DOWN.swap(down, Ordering::Relaxed) != down {
        crate::core::later(move |c| c.ptt(down));
    }
}

/// When the wheel last turned (epoch ms) while listening; 0 = not yet.
pub fn last() -> i64 {
    LAST.load(Ordering::Relaxed)
}

/// Self-test: as if the wheel had just turned.
pub fn test_turn() {
    LAST.store(rt::epoch_ms(), Ordering::Relaxed);
}

/// Listen to the wheel (the edge is being held, or the tab is out) or stop. Cheap to call often.
pub fn listen(on: bool) {
    if LISTENING.swap(on, Ordering::Relaxed) == on {
        return;
    }
    post_listen();
}

pub fn start() {
    let started = std::thread::Builder::new().name("chatdock-wheel".into()).spawn(|| unsafe {
        let class = w!("ChatDockWheel");
        let wc = WNDCLASSW { lpfnWndProc: Some(wheel_proc), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let hwnd =
            match CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) {
                Ok(h) => h,
                Err(err) => {
                    log!("wheel: no window: {err}");
                    return;
                }
            };
        WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
        register(hwnd); // what was asked for before the window was there
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
    if let Err(err) = started {
        log!("wheel: no thread: {err}");
    }
}

/// Copies of the mouse while the edge is held (or a side button talks); none otherwise. The
/// keyboard is never copied (a push-to-talk key is polled).
unsafe fn register(hwnd: HWND) {
    let ptt = PTT.load(Ordering::Relaxed);
    let on = LISTENING.load(Ordering::Relaxed) || ptt == MOUSE4 || ptt == MOUSE5;
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01, // generic desktop
        usUsage: 0x02,     // mouse
        dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
        hwndTarget: if on { hwnd } else { HWND::default() },
    };
    // (removing what was never registered fails harmlessly)
    if let Err(err) = RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32) {
        if on {
            log!("raw input mouse: {err}");
        }
    }
}

unsafe extern "system" fn wheel_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_LISTEN => {
            register(hwnd);
            LRESULT(0)
        }
        WM_INPUT => {
            let mut raw = RAWINPUT::default();
            let mut size = std::mem::size_of::<RAWINPUT>() as u32;
            let got = GetRawInputData(
                HRAWINPUT(lparam.0 as _),
                RID_INPUT,
                Some(&mut raw as *mut RAWINPUT as *mut _),
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            );
            let ptt = PTT.load(Ordering::Relaxed);
            if got != u32::MAX && got != 0 && raw.header.dwType == RIM_TYPEMOUSE.0 {
                let flags = raw.data.mouse.Anonymous.Anonymous.usButtonFlags as u32;
                if flags & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0 && LISTENING.load(Ordering::Relaxed) {
                    LAST.store(rt::epoch_ms(), Ordering::Relaxed);
                }
                if let Some(down) = ptt_edge(flags, ptt) {
                    ptt_change(down);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam) // lets Windows free the input
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn push_to_talk_keys() {
        assert_eq!(super::key_code("Mouse4"), Some(super::MOUSE4));
        assert_eq!(super::key_code("Mouse5"), Some(super::MOUSE5));
        assert_eq!(super::key_code("KeyV"), Some(0x56));
        assert_eq!(super::key_code("Digit3"), Some(0x33));
        assert_eq!(super::key_code("F13"), Some(0x7C));
        assert_eq!(super::key_code("F24"), Some(0x87));
        assert_eq!(super::key_code("Backquote"), Some(0xC0));
        assert_eq!(super::key_code("F25"), None);
        assert!(super::key_ok("KeyV") && super::key_ok("Mouse4") && super::key_ok("F13"));
        for refused in ["KeyW", "KeyA", "KeyS", "KeyD", "Space", "ShiftLeft", "ControlLeft", "Mouse1", "Escape", ""] {
            assert!(!super::key_ok(refused), "{refused}");
        }
    }
}

#[cfg(test)]
mod decode_tests {
    use super::{is_key, poll_edge, ptt_edge, MOUSE4, MOUSE5};

    #[test]
    fn push_to_talk_reads_its_own_button_only() {
        assert_eq!(ptt_edge(0x40, MOUSE4), Some(true));
        assert_eq!(ptt_edge(0x80, MOUSE4), Some(false));
        assert_eq!(ptt_edge(0x100, MOUSE5), Some(true));
        assert_eq!(ptt_edge(0x200, MOUSE5), Some(false));
        assert_eq!(ptt_edge(0x40, MOUSE5), None); // button 4 while 5 talks
        assert_eq!(ptt_edge(0x0400, MOUSE4), None); // the wheel
        assert_eq!(ptt_edge(0x40, 0x7C), None); // a mouse button while a key talks
        assert_eq!(ptt_edge(0x40, 0), None); // nothing chosen
    }

    #[test]
    fn a_polled_key_changes_once_per_press() {
        assert_eq!(poll_edge(false, true), Some(true)); // pressed
        assert_eq!(poll_edge(true, true), None); // still held
        assert_eq!(poll_edge(true, false), Some(false)); // let go
        assert_eq!(poll_edge(false, false), None);
        assert!(is_key(0x7C) && !is_key(MOUSE4) && !is_key(MOUSE5) && !is_key(0));
    }
}
