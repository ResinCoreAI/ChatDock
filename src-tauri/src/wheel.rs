//! The mouse wheel on the dock edge: turning it there means scrolling the page under the pointer (a
//! maximized browser's scrollbar sits right at the screen edge), not asking for the chat. A small
//! thread gets a copy of the mouse's raw input only while the edge is being held or the tab is out,
//! and notes when the wheel last turned. Raw input is a copy: it never holds up the mouse for
//! anything else, a game included.
//!
//! (Tauri's window layer registers the mouse for raw input for its device events, which ChatDock
//! doesn't use; a process has one registration per device kind, so this takes it over meanwhile.)

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, Ordering};

use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Input::{
                GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK,
                RIDEV_REMOVE, RID_INPUT, RIM_TYPEMOUSE,
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
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_LISTEN, WPARAM(on as usize), LPARAM(0));
        }
    }
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
        if LISTENING.load(Ordering::Relaxed) {
            register(hwnd, true); // asked for before the window was there
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
    if let Err(err) = started {
        log!("wheel: no thread: {err}");
    }
}

unsafe fn register(hwnd: HWND, on: bool) {
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01, // generic desktop
        usUsage: 0x02,     // mouse
        dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
        hwndTarget: if on { hwnd } else { HWND::default() },
    };
    if let Err(err) = RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32) {
        log!("wheel: raw input {}: {err}", if on { "on" } else { "off" });
    }
}

unsafe extern "system" fn wheel_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_LISTEN => {
            register(hwnd, wparam.0 != 0);
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
            if got != u32::MAX && got != 0 && raw.header.dwType == RIM_TYPEMOUSE.0 {
                let flags = raw.data.mouse.Anonymous.Anonymous.usButtonFlags as u32;
                if flags & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0 {
                    LAST.store(rt::epoch_ms(), Ordering::Relaxed);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam) // lets Windows free the input
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
