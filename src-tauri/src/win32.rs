//! Windows calls ChatDock needs that Tauri doesn't expose:
//!  - remember / give back the foreground window (closing the chat returns you to your game)
//!  - mouse buttons held, pointer hidden or held by a game, fullscreen apps and exclusive fullscreen
//!  - monitors (bounds, work area, scale, refresh rate) and moving / showing windows without
//!    taking focus. Positions here are physical pixels.

use windows::{
    core::{BOOL, HSTRING, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT, TRUE},
        Graphics::Gdi::{
            EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS, HDC, HMONITOR,
            MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONULL,
        },
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState, VK_CONTROL, VK_LBUTTON, VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_SHIFT},
            Shell::{
                SHQueryUserNotificationState, ShellExecuteW, QUERY_USER_NOTIFICATION_STATE, QUNS_PRESENTATION_MODE,
                QUNS_RUNNING_D3D_FULL_SCREEN,
            },
            WindowsAndMessaging::{
                BringWindowToTop, GetAncestor, GetClassNameW, GetClipCursor, GetCursorInfo, GetCursorPos, GetForegroundWindow,
                GetSystemMetrics, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow,
                IsWindowVisible, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowLongPtrW,
                SetWindowPos, ShowWindow, WindowFromPoint, CURSORINFO, CURSOR_SHOWING, GA_PARENT, GA_ROOTOWNER, GWL_EXSTYLE, GWL_STYLE,
                GW_CHILD, GW_HWNDNEXT, HWND_BOTTOM, HWND_TOPMOST, LWA_ALPHA, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
                SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOWNA,
                SW_SHOWNORMAL, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
            },
        },
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn from_win(r: RECT) -> Self {
        Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

pub fn h(hwnd: isize) -> HWND {
    HWND(hwnd as *mut std::ffi::c_void)
}

pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

pub fn root_owner(hwnd: isize) -> isize {
    if hwnd == 0 {
        return 0;
    }
    unsafe { GetAncestor(h(hwnd), GA_ROOTOWNER).0 as isize }
}

pub fn class_name(hwnd: isize) -> String {
    if hwnd == 0 {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h(hwnd), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn is_window(hwnd: isize) -> bool {
    hwnd != 0 && unsafe { IsWindow(Some(h(hwnd))).as_bool() }
}

pub fn is_visible(hwnd: isize) -> bool {
    hwnd != 0 && unsafe { IsWindowVisible(h(hwnd)).as_bool() }
}

/// SetForegroundWindow, falling back to the AttachThreadInput trick when Windows' focus-stealing
/// protection refuses the plain call. No synthetic key presses (those could leak into a game).
pub fn force_foreground(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    unsafe {
        if SetForegroundWindow(h(hwnd)).as_bool() {
            return true;
        }
        let fg = GetForegroundWindow();
        let fg_thread = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let me = GetCurrentThreadId();
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(h(hwnd));
        let ok = SetForegroundWindow(h(hwnd)).as_bool();
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        ok
    }
}

/// Give focus back to a window we took it from (un-minimizing exclusive-fullscreen games first).
pub fn restore_foreground(hwnd: isize) -> bool {
    if !is_window(hwnd) || !is_visible(hwnd) {
        return false;
    }
    unsafe {
        if IsIconic(h(hwnd)).as_bool() {
            let _ = ShowWindow(h(hwnd), SW_RESTORE);
        }
    }
    force_foreground(hwnd)
}

pub fn mouse_button_down() -> bool {
    unsafe { [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|vk| (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0) }
}

/// Modifier keys held right now: (ctrl, alt, shift)
pub fn modifiers() -> (bool, bool, bool) {
    let down = |vk: i32| unsafe { (GetKeyState(vk) as u16 & 0x8000) != 0 };
    (down(VK_CONTROL.0 as i32), down(VK_MENU.0 as i32), down(VK_SHIFT.0 as i32))
}

fn notification_state() -> QUERY_USER_NOTIFICATION_STATE {
    unsafe { SHQueryUserNotificationState().unwrap_or_default() }
}

pub fn is_exclusive_fullscreen() -> bool {
    notification_state() == QUNS_RUNNING_D3D_FULL_SCREEN
}

/// A game in exclusive fullscreen, or a presentation: nothing may appear over it.
pub fn nothing_may_show() -> bool {
    let s = notification_state();
    s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE
}

/// True when the foreground window covers its whole monitor (borderless / fullscreen game, F11 video, ...).
pub fn is_fullscreen_app_active() -> bool {
    let s = notification_state();
    if s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE {
        return true;
    }
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || !IsWindowVisible(fg).as_bool() {
            return false;
        }
        let class = class_name(fg.0 as isize);
        if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"].contains(&class.as_str()) {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
        if mon.0.is_null() {
            return false;
        }
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// The mouse pointer is hidden: a game in mouse-look mode (FPS aiming), a fullscreen video, ...
pub fn cursor_hidden() -> bool {
    let mut info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
    if unsafe { GetCursorInfo(&mut info) }.is_err() {
        return false;
    }
    (info.flags.0 & CURSOR_SHOWING.0) == 0 || info.hCursor.0.is_null()
}

/// Some program keeps the pointer inside part of the screen (ClipCursor), as games do while they
/// own the mouse. Unclipped, the clip rectangle is the whole virtual screen.
pub fn cursor_confined() -> bool {
    unsafe {
        let mut r = RECT::default();
        if GetClipCursor(&mut r).is_err() {
            return false;
        }
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let hh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let slack = 2;
        r.left > x + slack || r.top > y + slack || r.right < x + w - slack || r.bottom < y + hh - slack
    }
}

/// A game (or anything else) has taken the mouse: the pointer reaching the screen edge is then
/// just aiming or camera movement, not the user reaching for ChatDock.
pub fn mouse_captured() -> bool {
    cursor_hidden() || cursor_confined()
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

pub fn window_rect(hwnd: isize) -> Rect {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(h(hwnd), &mut r) };
    Rect::from_win(r)
}

/// The direct child windows of `parent`, top of the stack first.
pub fn children(parent: isize) -> Vec<isize> {
    let mut list = Vec::new();
    unsafe {
        let mut c = GetWindow(h(parent), GW_CHILD).ok();
        while let Some(w) = c.filter(|w| !w.0.is_null()) {
            list.push(w.0 as isize);
            c = GetWindow(w, GW_HWNDNEXT).ok();
        }
    }
    list
}

/// Keep a child window under its siblings, so they are drawn (and clicked) on top of it, and
/// stop it painting over them.
pub fn keep_at_bottom(hwnd: isize) {
    unsafe {
        let style = GetWindowLongPtrW(h(hwnd), GWL_STYLE);
        if style & WS_CLIPSIBLINGS.0 as isize == 0 {
            SetWindowLongPtrW(h(hwnd), GWL_STYLE, style | WS_CLIPSIBLINGS.0 as isize);
        }
        let below = GetWindow(h(hwnd), GW_HWNDNEXT).ok().is_some_and(|w| !w.0.is_null());
        if below {
            let _ = SetWindowPos(h(hwnd), Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }
}

/// Which direct child of `parent` is really on top at this screen point (0 = none): what a click
/// there would hit.
pub fn child_on_top_at(parent: isize, x: i32, y: i32) -> isize {
    unsafe {
        let mut w = WindowFromPoint(POINT { x, y });
        while !w.0.is_null() {
            let up = GetAncestor(w, GA_PARENT);
            if up.0 as isize == parent {
                return w.0 as isize;
            }
            w = up;
        }
    }
    0
}

/// See-through for the whole window, chats included (0 = invisible, 255 = solid). Once layered the
/// window stays layered, like Electron's: switching it back while visible makes the pages flash.
pub fn set_alpha(hwnd: isize, alpha: u8) {
    unsafe {
        let ex = GetWindowLongPtrW(h(hwnd), GWL_EXSTYLE);
        if ex & WS_EX_LAYERED.0 as isize == 0 {
            if alpha == 255 {
                return;
            }
            SetWindowLongPtrW(h(hwnd), GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        }
        let _ = SetLayeredWindowAttributes(h(hwnd), windows::Win32::Foundation::COLORREF(0), alpha, LWA_ALPHA);
    }
}

/// The file name of the program that owns a window ("" if unknown).
pub fn process_name(hwnd: isize) -> String {
    use windows::Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_WIN32};
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h(hwnd), Some(&mut pid));
        let Ok(proc) = windows::Win32::System::Threading::OpenProcess(
            windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        ) else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(proc);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or("").to_string()
    }
}

/// Windows is shutting down or signing out right now.
pub fn shutting_down() -> bool {
    unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_SHUTTINGDOWN) != 0 }
}

/// The window's real see-through level (None = not layered, i.e. solid).
pub fn layered_alpha(hwnd: isize) -> Option<u8> {
    use windows::Win32::UI::WindowsAndMessaging::{GetLayeredWindowAttributes, LAYERED_WINDOW_ATTRIBUTES_FLAGS};
    let mut alpha = 0u8;
    let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
    unsafe { GetLayeredWindowAttributes(h(hwnd), None, Some(&mut alpha), Some(&mut flags)).ok()? };
    (flags.0 & LWA_ALPHA.0 != 0).then_some(alpha)
}

/// Never listed in Alt+Tab or on the taskbar (the tab, pop-ups and other small windows).
pub fn set_tool_window(hwnd: isize) {
    unsafe {
        let ex = GetWindowLongPtrW(h(hwnd), GWL_EXSTYLE);
        SetWindowLongPtrW(h(hwnd), GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW.0 as isize);
    }
}

/// Show without taking focus (the game keeps its keyboard and mouse).
pub fn show_inactive(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(h(hwnd), SW_SHOWNA);
    }
}

pub fn show(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(h(hwnd), SW_SHOWNORMAL);
    }
}

pub fn hide(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(h(hwnd), SW_HIDE);
    }
}

/// Re-insert at the top of the topmost band (above games that are topmost themselves).
pub fn raise(hwnd: isize) {
    unsafe {
        let _ = SetWindowPos(h(hwnd), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

pub fn set_bounds(hwnd: isize, r: Rect) {
    unsafe {
        let _ = SetWindowPos(h(hwnd), None, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
        if window_rect(hwnd) != r {
            let _ = SetWindowPos(h(hwnd), None, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

pub fn move_to(hwnd: isize, x: i32, y: i32) {
    let before = window_rect(hwnd);
    unsafe {
        let _ = SetWindowPos(h(hwnd), None, x, y, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE);
    }
    let after = window_rect(hwnd);
    if after.w != before.w || after.h != before.h {
        set_bounds(hwnd, Rect { x, y, w: before.w, h: before.h });
    }
}

/// Keep the window out of screenshots, OBS and Discord screen share.
pub fn set_capture_excluded(hwnd: isize, on: bool) {
    unsafe {
        let _ = SetWindowDisplayAffinity(h(hwnd), if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE });
    }
}

/// Windows is set to dark mode for apps.
pub fn system_dark() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value = 1u32;
    let mut size = 4u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            &HSTRING::from("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:")) {
        return;
    }
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn show_in_folder(path: &std::path::Path) {
    let args = format!("/select,\"{}\"", path.display());
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from("explorer.exe"), &HSTRING::from(args), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

#[derive(Clone, Debug)]
pub struct Display {
    pub id: String,
    pub bounds: Rect,
    pub work: Rect,
    /// monitor scale: DIP -> physical px (panel widths, cursor distances)
    pub scale: f64,
    /// scale of ChatDock's own pages' CSS px: the monitor scale times Windows' "Text size" (WebView2
    /// renders pages with both)
    pub ui: f64,
    pub hz: u32,
    pub primary: bool,
}

unsafe extern "system" fn monitor_cb(mon: HMONITOR, _hdc: HDC, _rect: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<Display>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(mon, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        let id = String::from_utf16_lossy(&info.szDevice[..len]);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        let hz = if EnumDisplaySettingsW(&HSTRING::from(id.as_str()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
            dm.dmDisplayFrequency
        } else {
            60
        };
        list.push(Display {
            id,
            bounds: Rect::from_win(info.monitorInfo.rcMonitor),
            work: Rect::from_win(info.monitorInfo.rcWork),
            scale: dx as f64 / 96.0,
            ui: dx as f64 / 96.0 * text_scale(),
            hz: if hz > 1 { hz } else { 60 },
            primary: info.monitorInfo.dwFlags & 1 != 0,
        });
    }
    TRUE
}

static DISPLAYS: std::sync::Mutex<Option<(std::time::Instant, Vec<Display>)>> = std::sync::Mutex::new(None);

/// All monitors, primary first. Kept for a moment (the edge is checked on every screen refresh);
/// forget_displays() drops it as soon as Windows says something changed.
pub fn displays() -> Vec<Display> {
    let mut cache = DISPLAYS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed().as_millis() < 1000 {
            return list.clone();
        }
    }
    let list = enum_displays();
    *cache = Some((std::time::Instant::now(), list.clone()));
    list
}

pub fn forget_displays() {
    *DISPLAYS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *TEXT_SCALE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

static TEXT_SCALE: std::sync::Mutex<Option<f64>> = std::sync::Mutex::new(None);

/// Settings > Accessibility > Text size (1.0 - 2.25).
pub fn text_scale() -> f64 {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut cache = TEXT_SCALE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = *cache {
        return v;
    }
    let mut value = 0u32;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Accessibility"),
            &HSTRING::from("TextScaleFactor"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        )
        .is_ok()
    };
    let v = if ok && (100..=225).contains(&value) { value as f64 / 100.0 } else { 1.0 };
    *cache = Some(v);
    v
}

fn enum_displays() -> Vec<Display> {
    let mut list: Vec<Display> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_cb), LPARAM(&mut list as *mut Vec<Display> as isize));
    }
    list.sort_by_key(|d| (!d.primary, d.bounds.x, d.bounds.y));
    list
}
