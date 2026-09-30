//! Windows a chat opens itself: a call (Messenger, Instagram), "Sign in with Google / Apple", or a
//! blank page the site then fills in. WebView2 would make these on its own, outside everything
//! ChatDock does for the chat pages (the page script, the local-access block, the link, program and
//! permission rules, hiding from capture). ChatDock makes them instead: a window of its own with a
//! WebView2 in the app's profile, handed to the page through NewWindowRequested (SetNewWindow), so
//! the site still gets the window it asked for (window.opener, postMessage, close()). If that
//! can't be done, WebView2 makes the window as before.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{w, Interface, BOOL, HSTRING, PWSTR},
    Win32::{
        Foundation::{E_FAIL, HWND, LPARAM, LRESULT, RECT, WPARAM},
        UI::{HiDpi::AdjustWindowRectExForDpi, WindowsAndMessaging::*},
    },
};

use crate::{
    apps, chats,
    core::{later, Core},
    log, site_log, win32,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Call,
    SignIn,
    Blank,
}

impl Kind {
    fn says(self) -> &'static str {
        match self {
            Kind::Call => "a call",
            Kind::SignIn => "a sign-in",
            Kind::Blank => "a blank page",
        }
    }
}

struct Popup {
    app: String,
    hwnd: isize,
    kind: Kind,
    controller: Option<ICoreWebView2Controller>,
    /// the page's request, waiting for this window's WebView2 (its window.open() waits too)
    request: Option<(ICoreWebView2NewWindowRequestedEventArgs, ICoreWebView2Deferral)>,
    /// made by a click: the address a blank page is then given counts as clicked
    clicked: bool,
    /// a sign-in follows the sign-in wherever it goes (https)
    sign_in: bool,
    /// still the blank page it began as
    blank: bool,
    /// full screen (a call's video): the window's style and place before
    before_full: Option<(isize, win32::Rect)>,
}

type Request = (ICoreWebView2NewWindowRequestedEventArgs, ICoreWebView2Deferral);

thread_local! {
    static POPUPS: RefCell<Vec<Popup>> = const { RefCell::new(Vec::new()) };
    /// Requests on their way to open() (later() only carries what can cross threads)
    static WAITING: RefCell<HashMap<u64, Request>> = RefCell::new(HashMap::new());
    static SEQ: Cell<u64> = const { Cell::new(0) };
}

const CLASS: windows::core::PCWSTR = w!("ChatDockPopup");

/// A page asked for a window of its own (see chats::new_window): hold its request, and make the
/// window.
pub unsafe fn requested(
    app: &str,
    kind: Kind,
    clicked: bool,
    args: &ICoreWebView2NewWindowRequestedEventArgs,
) -> windows::core::Result<()> {
    let deferral = args.GetDeferral()?;
    let key = SEQ.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    WAITING.with(|w| w.borrow_mut().insert(key, (args.clone(), deferral)));
    let a = app.to_string();
    later(move |c| {
        site_log!(&a, "{a} opens a window of its own: {}", kind.says());
        open(c, key, &a, kind, clicked);
    });
    Ok(())
}

fn open(c: &mut Core, key: u64, app: &str, kind: Kind, clicked: bool) {
    let Some(request) = WAITING.with(|w| w.borrow_mut().remove(&key)) else { return };
    match unsafe { make(c, app, kind, clicked, request) } {
        Ok(hwnd) => c.own_hwnds.push(hwnd),
        Err((err, request)) => {
            log!("{app}: its window could not be made ({err}), WebView2 makes it");
            if let Some((_, deferral)) = request {
                let _ = unsafe { deferral.Complete() }; // not handled: WebView2's own window
            }
            if kind == Kind::Call {
                c.call_window_opening(app);
            }
        }
    }
}

type Failed = (windows::core::Error, Option<Request>);

/// The window, and its WebView2 on the way (ready()).
unsafe fn make(c: &Core, app: &str, kind: Kind, clicked: bool, request: Request) -> std::result::Result<isize, Failed> {
    let fail = |err: windows::core::Error, request: Request| (err, Some(request));
    let Some(env) = chats::environment() else { return Err(fail(E_FAIL.into(), request)) };
    let env10 = match env.cast::<ICoreWebView2Environment10>() {
        Ok(e) => e,
        Err(err) => return Err(fail(err, request)),
    };
    let r = placement(&request.0, kind);
    let name = apps::get(app).map(|a| a.name).unwrap_or(app);
    let hwnd = match create_window(name, r) {
        Ok(h) => h,
        Err(err) => return Err(fail(err, request)),
    };
    // the app's own icon, like its other windows
    for which in [ICON_BIG, ICON_SMALL] {
        let icon = SendMessageW(win32::h(c.panel.hwnd), WM_GETICON, Some(WPARAM(which as usize)), Some(LPARAM(0)));
        if icon.0 != 0 {
            SendMessageW(win32::h(hwnd), WM_SETICON, Some(WPARAM(which as usize)), Some(LPARAM(icon.0)));
        }
    }
    win32::set_capture_excluded(hwnd, c.settings.bool("hideFromCapture"));
    POPUPS.with(|p| {
        p.borrow_mut().push(Popup {
            app: app.to_string(),
            hwnd,
            kind,
            controller: None,
            request: Some(request),
            clicked,
            sign_in: kind == Kind::SignIn,
            blank: kind == Kind::Blank,
            before_full: None,
        })
    });
    let (debug, selftest, dark) = (c.args.dev_tools(), c.args.selftest, c.chats.dark());
    let made = (|| -> windows::core::Result<()> {
        let opts = env10.CreateCoreWebView2ControllerOptions()?;
        opts.SetProfileName(&HSTRING::from(app))?; // the page's own profile (SetNewWindow needs it)
        opts.SetIsInPrivateModeEnabled(false)?;
        if let Ok(o3) = opts.cast::<ICoreWebView2ControllerOptions3>() {
            let _ = o3.SetDefaultBackgroundColor(chats::color(dark));
        }
        env10.CreateCoreWebView2ControllerWithOptions(
            win32::h(hwnd),
            &opts,
            &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |err, controller| {
                match (err, controller) {
                    (Ok(()), Some(controller)) => ready(hwnd, controller, debug, selftest),
                    (err, _) => gave_up(hwnd, &format!("{err:?}")),
                }
                Ok(())
            })),
        )
    })();
    match made {
        Ok(()) => Ok(hwnd),
        Err(err) => {
            let request = forget(hwnd).and_then(|p| p.request);
            let _ = DestroyWindow(win32::h(hwnd));
            Err((err, request))
        }
    }
}

/// Its WebView2 is up: the same settings and page script as the chats, then the page.
unsafe fn ready(hwnd: isize, controller: ICoreWebView2Controller, debug: bool, selftest: bool) {
    let kept =
        POPUPS.with(|p| p.borrow_mut().iter_mut().find(|p| p.hwnd == hwnd).map(|p| p.controller = Some(controller.clone())).is_some());
    if !kept {
        let _ = controller.Close(); // closed meanwhile
        return;
    }
    let _ = controller.SetBounds(client_rect(hwnd));
    let set = (|| -> windows::core::Result<ICoreWebView2> {
        let wv = controller.CoreWebView2()?;
        chats::apply_settings(&wv, debug, false)?;
        Ok(wv)
    })();
    let wv = match set {
        Ok(wv) => wv,
        Err(err) => return gave_up(hwnd, &err.to_string()),
    };
    // settings and the page script first (SetNewWindow wants the script in already), then the page
    let page = wv.clone();
    let added = wv.AddScriptToExecuteOnDocumentCreated(
        &HSTRING::from(chats::site_script(selftest)),
        &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |_, _| {
            hand_over(hwnd, &page, selftest);
            Ok(())
        })),
    );
    if let Err(err) = added {
        gave_up(hwnd, &err.to_string());
    }
}

/// The page gets this window; the rules that must come after that go on, and it shows.
unsafe fn hand_over(hwnd: isize, wv: &ICoreWebView2, selftest: bool) {
    let Some((app, kind, Some((args, deferral)))) =
        POPUPS.with(|p| p.borrow_mut().iter_mut().find(|p| p.hwnd == hwnd).map(|p| (p.app.clone(), p.kind, p.request.take())))
    else {
        return;
    };
    let given = args.SetNewWindow(wv).and_then(|_| args.SetHandled(true));
    if given.is_ok() {
        if let Err(err) = guard(wv, &app, hwnd, selftest) {
            log!("{app}: its window's rules: {err}");
        }
    }
    let _ = deferral.Complete();
    match given {
        Ok(()) => {
            if selftest {
                let _ = ShowWindow(win32::h(hwnd), SW_SHOWNOACTIVATE); // (the test doesn't take the keyboard)
            } else {
                win32::show(hwnd);
                let _ = SetForegroundWindow(win32::h(hwnd));
            }
            later(move |c| c.popup_shown(hwnd, &app, kind == Kind::Call));
        }
        Err(err) => {
            // (WebView2 made it itself, or the page is gone)
            log!("{app}: its window wasn't taken ({err})");
            close(hwnd);
            if kind == Kind::Call {
                later(move |c| c.call_window_opening(&app));
            }
        }
    }
}

/// Its WebView2 couldn't be made: WebView2 makes the window itself, as before.
unsafe fn gave_up(hwnd: isize, why: &str) {
    let Some(p) = forget(hwnd) else { return };
    log!("{}: its window's page could not be made ({why}), WebView2 makes it", p.app);
    if let Some((_, deferral)) = p.request {
        let _ = deferral.Complete();
    }
    if let Some(c) = p.controller {
        let _ = c.Close();
    }
    let _ = DestroyWindow(win32::h(hwnd));
    if p.kind == Kind::Call {
        let app = p.app.clone();
        later(move |c| c.call_window_opening(&app));
    }
}

/// What the chat pages have, in this window too (see chats::configure), and the window's own: where
/// its page may go, closing, its title, full screen.
unsafe fn guard(wv: &ICoreWebView2, app: &str, hwnd: isize, selftest: bool) -> windows::core::Result<()> {
    chats::block_this_pc(wv)?;
    chats::guard_new_windows(wv, app)?;
    chats::guard_programs(wv, app, selftest);
    chats::guard_permissions(wv, app)?;
    let mut token = 0i64;

    // a call stays with its app, a sign-in with its sign-in; a blank page the site sends somewhere
    // else was only there for a link: the link goes to the browser (if it was clicked) and the
    // window goes
    let a = app.to_string();
    wv.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            args.Uri(&mut p)?;
            let uri = take_pwstr(p);
            let Some((blank, sign_in, made_by_click)) =
                POPUPS.with(|p| p.borrow().iter().find(|p| p.hwnd == hwnd).map(|p| (p.blank, p.sign_in, p.clicked)))
            else {
                return Ok(());
            };
            if uri.starts_with("about:") {
                return Ok(());
            }
            if (sign_in && uri.starts_with("https://")) || apps::keep_inside(&a, &uri) || own_blob(&a, &uri) {
                let signs_in = blank && apps::is_auth_popup(&a, &uri);
                POPUPS.with(|p| {
                    if let Some(p) = p.borrow_mut().iter_mut().find(|p| p.hwnd == hwnd) {
                        p.blank = false;
                        p.sign_in |= signs_in;
                    }
                });
                return Ok(());
            }
            args.SetCancel(true)?;
            let mut clicked = BOOL(0);
            let _ = args.IsUserInitiated(&mut clicked);
            let clicked = clicked.as_bool() || (blank && made_by_click);
            let (app, what) = (a.clone(), chats::outside_kind(&uri));
            if chats::opens_outside(&uri, clicked) {
                later(move |c| {
                    site_log!(&app, "{app}: {what} opens outside ChatDock (from a window of its own)");
                    c.open_external(&app, &uri);
                });
            } else {
                later(move |_| site_log!(&app, "{app}: {what} not opened (nobody clicked it, in a window of its own)"));
            }
            if blank {
                let _ = PostMessageW(Some(win32::h(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            Ok(())
        })),
        &mut token,
    )?;

    // the site closes it (window.close())
    wv.add_WindowCloseRequested(
        &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
            let _ = PostMessageW(Some(win32::h(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0));
            Ok(())
        })),
        &mut token,
    )?;

    let name = apps::get(app).map(|a| a.name).unwrap_or(app).to_string();
    wv.add_DocumentTitleChanged(
        &DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut p = PWSTR::null();
                sender.DocumentTitle(&mut p)?;
                let title = take_pwstr(p);
                let title = if title.trim().is_empty() { name.clone() } else { crate::core::clean_text(&title, 200) };
                let _ = SetWindowTextW(win32::h(hwnd), &HSTRING::from(title));
            }
            Ok(())
        })),
        &mut token,
    )?;

    wv.add_ContainsFullScreenElementChanged(
        &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(sender) = sender {
                let mut full = BOOL(0);
                sender.ContainsFullScreenElement(&mut full)?;
                set_full_screen(hwnd, full.as_bool());
            }
            Ok(())
        })),
        &mut token,
    )?;

    // the page script's reports (a passkey refused); nothing else from here is read
    let a = app.to_string();
    wv.add_WebMessageReceived(
        &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            if args.TryGetWebMessageAsString(&mut p).is_ok() {
                let msg = take_pwstr(p);
                if msg.len() <= chats::MESSAGE_MAX {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg) {
                        if v["type"] == "passkey" {
                            site_log!(&a, "passkey request blocked {a} (in a window of its own): {}", chats::passkey_line(&v));
                        }
                    }
                }
            }
            Ok(())
        })),
        &mut token,
    )?;
    Ok(())
}

/// A blob: address of the app's own site (a file it shows in the window).
fn own_blob(app: &str, uri: &str) -> bool {
    uri.strip_prefix("blob:").is_some_and(|inner| apps::owns(app, inner))
}

/// The size the page asked for (else one that suits the kind), in the middle of the screen with the
/// mouse. Physical pixels, the window's frame included.
unsafe fn placement(args: &ICoreWebView2NewWindowRequestedEventArgs, kind: Kind) -> win32::Rect {
    let (cx, cy) = win32::cursor_pos();
    let displays = win32::displays();
    let d = displays.iter().find(|d| d.bounds.contains(cx, cy)).or(displays.first());
    let (work, scale) = d.map(|d| (d.work, d.scale)).unwrap_or((win32::Rect { x: 0, y: 0, w: 1280, h: 720 }, 1.0));
    let (mut w, mut h) = match kind {
        Kind::Call => (1000u32, 700u32),
        Kind::SignIn => (520, 700),
        Kind::Blank => (900, 700),
    };
    if let Ok(f) = args.WindowFeatures() {
        let mut has = BOOL(0);
        if f.HasSize(&mut has).is_ok() && has.as_bool() {
            let (mut fw, mut fh) = (0u32, 0u32);
            if f.Width(&mut fw).is_ok() && f.Height(&mut fh).is_ok() && fw >= 200 && fh >= 150 {
                (w, h) = (fw, fh);
            }
        }
    }
    let mut r = RECT { left: 0, top: 0, right: (w as f64 * scale) as i32, bottom: (h as f64 * scale) as i32 };
    let _ = AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, false, WS_EX_APPWINDOW, (96.0 * scale) as u32);
    let (w, h) = ((r.right - r.left).min(work.w), (r.bottom - r.top).min(work.h));
    win32::Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h }
}

unsafe fn create_window(title: &str, r: win32::Rect) -> windows::core::Result<isize> {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(popup_proc),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&wc);
    });
    let hwnd = CreateWindowExW(
        WS_EX_APPWINDOW,
        CLASS,
        &HSTRING::from(title),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        r.x,
        r.y,
        r.w,
        r.h,
        None,
        None,
        None,
        None,
    )?;
    Ok(hwnd.0 as isize)
}

fn client_rect(hwnd: isize) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(win32::h(hwnd), &mut r);
    }
    r
}

fn controller_of(hwnd: isize) -> Option<ICoreWebView2Controller> {
    POPUPS.with(|p| p.borrow().iter().find(|p| p.hwnd == hwnd).and_then(|p| p.controller.clone()))
}

/// Out of the list (its window is going): what it held.
fn forget(hwnd: isize) -> Option<Popup> {
    POPUPS.with(|p| {
        let mut list = p.borrow_mut();
        let i = list.iter().position(|p| p.hwnd == hwnd)?;
        Some(list.remove(i))
    })
}

/// Close it: its page first, then the window. A page still waiting for it gets no window.
unsafe fn close(hwnd: isize) {
    let (controller, request) = POPUPS.with(|p| {
        p.borrow_mut().iter_mut().find(|p| p.hwnd == hwnd).map(|p| (p.controller.take(), p.request.take())).unwrap_or((None, None))
    });
    if let Some((args, deferral)) = request {
        let _ = args.SetHandled(true);
        let _ = deferral.Complete();
    }
    if let Some(c) = controller {
        let _ = c.Close();
    }
    let _ = DestroyWindow(win32::h(hwnd));
}

unsafe fn set_full_screen(hwnd: isize, on: bool) {
    let h = win32::h(hwnd);
    if on {
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        let before = win32::window_rect(hwnd);
        let fresh = POPUPS.with(|p| {
            p.borrow_mut()
                .iter_mut()
                .find(|p| p.hwnd == hwnd && p.before_full.is_none())
                .map(|p| p.before_full = Some((style, before)))
                .is_some()
        });
        if !fresh {
            return;
        }
        let (mx, my) = (before.x + before.w / 2, before.y + before.h / 2);
        let displays = win32::displays();
        let Some(d) = displays.iter().find(|d| d.bounds.contains(mx, my)).or(displays.first()) else { return };
        SetWindowLongPtrW(h, GWL_STYLE, (style & !(WS_OVERLAPPEDWINDOW.0 as isize)) | WS_POPUP.0 as isize);
        let b = d.bounds;
        let _ = SetWindowPos(h, Some(HWND_TOP), b.x, b.y, b.w, b.h, SWP_FRAMECHANGED | SWP_NOOWNERZORDER);
    } else if let Some((style, r)) = POPUPS.with(|p| p.borrow_mut().iter_mut().find(|p| p.hwnd == hwnd).and_then(|p| p.before_full.take()))
    {
        SetWindowLongPtrW(h, GWL_STYLE, style);
        let _ = SetWindowPos(h, None, r.x, r.y, r.w, r.h, SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOOWNERZORDER);
    }
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let id = hwnd.0 as isize;
    match msg {
        WM_SIZE => {
            if let Some(c) = controller_of(id) {
                let _ = c.SetBounds(client_rect(id));
            }
            LRESULT(0)
        }
        WM_MOVE => {
            if let Some(c) = controller_of(id) {
                let _ = c.NotifyParentWindowPositionChanged();
            }
            LRESULT(0)
        }
        WM_SETFOCUS => {
            if let Some(c) = controller_of(id) {
                let _ = c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let r = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            LRESULT(0)
        }
        WM_CLOSE => {
            close(id);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            if let Some(p) = forget(id) {
                if let Some((args, deferral)) = p.request {
                    let _ = args.SetHandled(true); // (gone before its page came: no window)
                    let _ = deferral.Complete();
                }
                if let Some(c) = p.controller {
                    let _ = c.Close();
                }
            }
            later(move |c| c.popup_closed(id));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// ChatDock's windows for the chats (hiding from capture).
pub fn hwnds() -> Vec<isize> {
    POPUPS.with(|p| p.borrow().iter().map(|p| p.hwnd).collect())
}

/// The app's windows go with its page (switched off, put to sleep, its data cleared).
pub fn close_app(app: &str) {
    let theirs: Vec<isize> = POPUPS.with(|p| p.borrow().iter().filter(|p| p.app == app).map(|p| p.hwnd).collect());
    for hwnd in theirs {
        unsafe { close(hwnd) };
    }
}
