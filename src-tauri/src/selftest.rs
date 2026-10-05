//! Self-test: `ChatDock.exe --selftest --profile=<test dir> [--shots=<dir>] [--keep] [--selftest-only=edge]`
//! It drives the running app from a thread of its own (every step reaches the app state through
//! later()) and logs what it saw, followed by "(expect …)". Run it on a test profile only: it
//! switches settings around. `--selftest-only=edge` never takes focus, so it can run during a game.

use std::{
    path::PathBuf,
    sync::{atomic::Ordering, mpsc},
    time::Duration,
};

use serde_json::json;
use tauri::Manager;
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, CapturePreviewCompletedHandler, ExecuteScriptCompletedHandler,
    Microsoft::Web::WebView2::Win32::*,
};
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{LPARAM, WPARAM},
        Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
        },
        System::Com::{STGM_CREATE, STGM_WRITE},
        UI::{
            Shell::SHCreateStreamOnFileEx,
            WindowsAndMessaging::{GetWindowDisplayAffinity, PostMessageW, SendMessageW, WM_CLOSE, WM_ENDSESSION, WM_QUERYENDSESSION},
        },
    },
};

use crate::{
    core::{later, Core, PanelState, SESSION_ENDING},
    frames, i18n, log, rt, win32,
};

pub fn start() {
    let _ = std::thread::Builder::new().name("chatdock-selftest".into()).spawn(|| {
        let only = on(|c| c.args.selftest_only.clone());
        log!("selftest: hotkey ok = {} | apps {:?}", on(|c| c.hotkey_ok), on(|c| c.enabled_apps()));
        if only.as_deref() == Some("edge") {
            wait(3000);
            edge_test(false);
        } else if only.as_deref() == Some("fixes") {
            wait(9000); // the chats load
            fixes_test();
        } else if only.as_deref() == Some("whatsnew") {
            wait(3000);
            whats_new_test();
        } else if only.as_deref() == Some("settings") {
            wait(3000);
            settings_shots();
        } else if only.as_deref() == Some("counts") {
            wait(3000);
            counts_test();
        } else if only.as_deref() == Some("share") {
            share_test();
        } else if only.as_deref() == Some("discord") {
            discord_test();
        } else if only.as_deref() == Some("calls") {
            wait(7000); // the pages load
            call_test();
            call_window_test();
        } else if only.as_deref() == Some("inbox") {
            wait(8000); // the pages load
            inbox_test();
        } else if only.as_deref() == Some("fixes173") {
            wait(8000); // the pages load
            fixes173_test();
        } else if only.as_deref() == Some("fixes172") {
            wait(8000); // the pages load
            fixes172_test();
        } else if only.as_deref() == Some("security") {
            wait(8000); // the pages load
            security_test();
        } else if only.as_deref() == Some("updater") {
            wait(4000); // ChatDock's own pages load
            updater_test();
        } else if only.as_deref() == Some("review171") {
            wait(8000); // the pages load
            review171_test();
        } else if only.as_deref() == Some("ui171") {
            wait(8000); // the pages load
            ui171_test();
        } else if only.as_deref() == Some("notif") {
            wait(8000); // the pages load
            notification_path_test();
        } else if only.as_deref() == Some("volume") {
            wait(8000); // the pages load
            volume_test();
            spotify_test();
        } else if only.as_deref() == Some("drm") {
            wait(8000); // the pages load
            drm_test();
        } else if only.as_deref() == Some("review") {
            wait(7000); // the pages load
            review_fixes_test();
        } else if only.as_deref() == Some("newsgif") {
            wait(3000);
            news_gif_frames();
        } else if only.as_deref() == Some("tour") {
            wait(4000); // ChatDock's own pages load
            tour_test();
        } else if only.as_deref() == Some("gamemode") {
            wait(8000); // the pages load
            game_mode_test();
            facebook_count_test();
        } else if only.as_deref() == Some("gameintro") {
            wait(4000); // ChatDock's own pages load
            game_intro_test();
        } else if only.as_deref() == Some("tourgif") {
            wait(4000);
            tour_gif_frames();
        } else if only.as_deref() == Some("newsdemo") {
            wait(3000);
            news_demo_test();
        } else if only.as_deref() == Some("perf") {
            perf_test();
        } else if only.as_deref() == Some("sharebar") {
            wait(5000); // the chats' browser is up
            share_bar_test();
        } else if only.as_deref() == Some("view") {
            wait(9000); // the chats load
            for id in on(|c| c.enabled_apps()) {
                on(move |c| c.open_panel(Some(id), "selftest"));
                wait(1500);
                view_check(&format!("40-view-{id}"));
            }
            click_check();
            close_panel();
        } else {
            full_test();
        }
        log!("selftest done");
        if !std::env::args().any(|a| a == "--keep") {
            on(|c| c.quit());
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------
fn wait(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Run f with the app state on the main thread and hand back what it returns.
fn on<R: Send + 'static>(f: impl FnOnce(&mut Core) -> R + Send + 'static) -> R {
    let (tx, rx) = mpsc::channel();
    later(move |c| {
        let _ = tx.send(f(c));
    });
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(r) => r,
        Err(_) => {
            log!("selftest: the app stopped answering (main thread stuck?)");
            std::process::exit(3);
        }
    }
}

/// A script in one of ChatDock's own pages; its result as JSON text.
fn page_js(label: &str, js: &str) -> String {
    let Some(w) = rt::app().get_webview_window(label) else { return "err:no such window".into() };
    let (tx, rx) = mpsc::channel::<String>();
    let js = js.to_string();
    let sent = w.with_webview(move |pw| unsafe {
        match pw.controller().CoreWebView2() {
            Ok(wv) => {
                let done = tx.clone();
                let handler = ExecuteScriptCompletedHandler::create(Box::new(move |_, result| {
                    let _ = done.send(result);
                    Ok(())
                }));
                if wv.ExecuteScript(&HSTRING::from(js), &handler).is_err() {
                    let _ = tx.send("err:ExecuteScript".into());
                }
            }
            Err(e) => {
                let _ = tx.send(format!("err:{e}"));
            }
        }
    });
    if sent.is_err() {
        return "err:with_webview".into();
    }
    rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| "err:timeout".into())
}

/// A script in a chat site's page; its result as JSON text.
fn view_js(id: &str, js: &str) -> String {
    let (tx, rx) = mpsc::channel::<String>();
    let (id, js) = (id.to_string(), js.to_string());
    later(move |c| {
        c.chats.execute(&id, &js, move |r| {
            let _ = tx.send(r);
        })
    });
    rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| "err:no answer".into())
}

/// Same, for an expression that gives a promise (ExecuteScript doesn't wait for promises).
fn view_js_async(id: &str, expr: &str) -> String {
    view_js(
        id,
        &format!("window.__cdTest = 'pending'; (async () => {{ try {{ window.__cdTest = JSON.stringify(await ({expr})); }} catch (e) {{ window.__cdTest = 'err:' + e.message; }} }})(); true"),
    );
    for _ in 0..20 {
        wait(150);
        let r = view_js(id, "window.__cdTest");
        if r != "\"pending\"" {
            return serde_json::from_str::<String>(&r).unwrap_or(r);
        }
    }
    "err:timeout".into()
}

fn js_click(label: &str, selector: &str) -> String {
    page_js(
        label,
        &format!("(() => {{ const el = document.querySelector({}); if (!el) return false; el.click(); return true; }})()", json!(selector)),
    )
}

unsafe fn capture(wv: &ICoreWebView2, path: PathBuf, tx: mpsc::Sender<String>) {
    let stream = match SHCreateStreamOnFileEx(&HSTRING::from(path.as_os_str()), STGM_CREATE.0 | STGM_WRITE.0, 0x80, true, None) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(format!("err:{e}"));
            return;
        }
    };
    let keep = stream.clone();
    let done = tx.clone();
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |r| {
        drop(keep); // closes the file
        let _ = done.send(if r.is_ok() { "ok".into() } else { "err:capture".into() });
        Ok(())
    }));
    if wv.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler).is_err() {
        let _ = tx.send("err:CapturePreview".into());
    }
}

/// Each visible ChatDock page (and the chat on screen) saved as <shots>/<name>-<page>.png.
fn shot(name: &str) {
    let Some(dir) = on(|c| c.args.shots.clone()) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let (pages, view) = on(|c| {
        let mut pages = Vec::new();
        for (label, hwnd) in
            [("tab", c.tab.hwnd), ("panel", c.panel.hwnd), ("glow", c.glow.hwnd), ("edge", c.edgewin.hwnd), ("toasts", c.toastwin.hwnd)]
        {
            if win32::is_visible(hwnd) {
                pages.push(label);
            }
        }
        if c.update_win.as_ref().is_some_and(|w| win32::is_visible(w.hwnd)) {
            pages.push("update");
        }
        if c.whatsnew.win.as_ref().is_some_and(|w| win32::is_visible(w.hwnd)) {
            pages.push("whatsnew");
        }
        let active = c.active();
        let view = (win32::is_visible(c.panel.hwnd) && c.chats.is_visible(&active)).then_some(active);
        (pages, view)
    });
    let mut out = Vec::new();
    for label in pages {
        let Some(w) = rt::app().get_webview_window(label) else { continue };
        let (tx, rx) = mpsc::channel::<String>();
        let path = dir.join(format!("{name}-{label}.png"));
        let _ = w.with_webview(move |pw| unsafe {
            if let Ok(wv) = pw.controller().CoreWebView2() {
                capture(&wv, path, tx);
            }
        });
        out.push(format!("{label}:{}", rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| "timeout".into())));
    }
    if let Some(id) = view {
        let (tx, rx) = mpsc::channel::<String>();
        let path = dir.join(format!("{name}-view.png"));
        later(move |c| {
            if let Some(wv) = c.chats.webview(&id) {
                unsafe { capture(&wv, path, tx) };
            }
        });
        out.push(format!("view:{}", rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| "timeout".into())));
    }
    log!("shot {name} {}", out.join(" "));
}

/// What is really on screen where the chat should be: which window a click in the middle of it
/// would hit, and how much of it is still the empty panel background (a real screen capture, not
/// the page's own rendering; black while the PC is locked).
fn view_check(name: &str) {
    let (id, panel, page, center, area, visible) = on(|c| {
        let id = c.active();
        let p = win32::window_rect(c.panel.hwnd);
        let b = c.chats.bounds(&id).unwrap_or_default();
        let area = win32::Rect { x: p.x + b.x, y: p.y + b.y, w: b.w, h: b.h };
        (id.clone(), c.panel.hwnd, c.panel_page_hwnds.clone(), (area.x + area.w / 2, area.y + area.h / 2), area, c.chats.is_visible(&id))
    });
    let top = win32::child_on_top_at(panel, center.0, center.1);
    let hit = if top == 0 {
        "nothing".to_string()
    } else if page.contains(&top) {
        "the panel page".to_string()
    } else {
        format!("a chat window ({})", win32::class_name(top))
    };
    let bg = on(|c| {
        let theme = c.settings.str("theme").to_string();
        if theme == "light" || (theme != "dark" && !win32::system_dark()) {
            (255, 255, 255)
        } else {
            (24, 25, 29)
        }
    });
    let (blank, dark) = screen_grab(name, area, bg);
    log!(
        "view {id}: panel {:?} | visible {visible} | on top at its centre: {hit} | empty panel background {blank}% of it | black {dark}% (expect Open, true, a chat window, low %; black 100% only while locked)",
        panel_state()
    );
}

/// A real click in the middle of the chat (the pointer goes straight back): the click must reach
/// the page, and the panel must stay open and in front (it hides when you click anything else).
fn click_check() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
    let (id, center, before) = on(|c| {
        let id = c.active();
        let p = win32::window_rect(c.panel.hwnd);
        let b = c.chats.bounds(&id).unwrap_or_default();
        (id, (p.x + b.x + b.w / 2, p.y + b.y + b.h / 3), c.panel_state)
    });
    if before != PanelState::Open {
        log!("click into {id}: skipped, the panel is {before:?} (something else took the focus)");
        return;
    }
    view_js(&id, "window.__cdFocus = 0; window.addEventListener('focus', () => { window.__cdFocus++; }); document.addEventListener('mousedown', () => { window.__cdDown = (window.__cdDown || 0) + 1; }, true); true");
    let was = win32::cursor_pos();
    let click = |flags| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } } };
    unsafe {
        let _ = SetCursorPos(center.0, center.1);
        SendInput(&[click(MOUSEEVENTF_LEFTDOWN), click(MOUSEEVENTF_LEFTUP)], std::mem::size_of::<INPUT>() as i32);
    }
    wait(60);
    unsafe {
        let _ = SetCursorPos(was.0, was.1);
    }
    wait(500);
    let (state, front) = on(|c| (c.panel_state, win32::foreground_window() == c.panel.hwnd));
    log!(
        "click into {id}: page got the mouse {} | page has focus {} | panel {state:?} | panel in front {front} (expect 1, true, Open, true)",
        view_js(&id, "window.__cdDown || 0"),
        view_js(&id, "document.hasFocus()")
    );
}

/// Screen pixels of `r` saved as <shots>/<name>-screen.bmp; returns how much of it is the empty
/// panel background (dark or light theme) and how much is black, in %.
fn screen_grab(name: &str, r: win32::Rect, bg: (u8, u8, u8)) -> (u32, u32) {
    if r.w <= 0 || r.h <= 0 {
        return (0, 0);
    }
    let mut px = vec![0u8; (r.w * r.h * 4) as usize];
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, r.w, r.h);
        let old = SelectObject(mem, bmp.into());
        let _ = BitBlt(mem, 0, 0, r.w, r.h, Some(screen), r.x, r.y, SRCCOPY | CAPTUREBLT);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r.w,
                biHeight: -r.h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        GetDIBits(mem, bmp, 0, r.h as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
    let n = (r.w * r.h) as usize;
    let bg_color = bg;
    let (mut bg, mut black) = (0usize, 0usize);
    for p in px.chunks_exact(4) {
        let (b, g, rr) = (p[0], p[1], p[2]);
        if (rr, g, b) == bg_color {
            bg += 1;
        }
        if (rr, g, b) == (0, 0, 0) {
            black += 1;
        }
    }
    if let Some(dir) = on(|c| c.args.shots.clone()) {
        let _ = std::fs::create_dir_all(&dir);
        let mut file = Vec::with_capacity(54 + px.len());
        let size = 54 + px.len() as u32;
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&size.to_le_bytes());
        file.extend_from_slice(&[0, 0, 0, 0]);
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&r.w.to_le_bytes());
        file.extend_from_slice(&(-r.h).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&[0u8; 24]);
        file.extend_from_slice(&px);
        let _ = std::fs::write(dir.join(format!("{name}-screen.bmp")), file);
    }
    ((bg * 100 / n) as u32, (black * 100 / n) as u32)
}

fn set_cursor(x: i32, y: i32) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y);
    }
}

fn log_has(text: &str) -> bool {
    crate::log::path().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|s| s.contains(text))
}

/// The 1.5.2 fixes, one check each.
fn fixes_test() {
    let first = on(|c| c.enabled_apps()[0]);
    // (Something else may take the focus while this runs, which hides the panel: open it again.)
    let ensure_open = move || {
        if panel_state() != PanelState::Open {
            on(move |c| c.open_panel(Some(first), "selftest"));
            wait(700);
        }
    };
    // Pop-ups already on screen move beside the panel when it opens (they used to stay over it).
    // (A ChatDock notice: a pop-up of the app being opened goes away by design.)
    close_panel();
    on(|c| c.notice("ChatDock self-test", "this pop-up should move beside the panel"));
    wait(900);
    on(move |c| c.open_panel(Some(first), "selftest"));
    wait(800);
    let (shown, t, p, ui, left) =
        on(|c| (c.toasts_visible(), c.toasts_bounds(), win32::window_rect(c.panel.hwnd), c.target_display().ui, c.on_left()));
    let pad = (16.0 * ui).round() as i32 + 1;
    let beside = if left { t.x >= p.right() - pad } else { t.right() <= p.x + pad };
    log!("pop-up beside the open panel: shown {shown} | beside {beside} (pop-ups {t:?}, panel {p:?}) (expect true, true)");
    on(|c| c.toasts_dismiss_all());
    ensure_open();

    // Chat window opacity is applied (whole window, chat included), and back to solid.
    let alpha = on(|c| {
        c.set_pref("opacity", json!(0.7));
        (c.panel_alpha, win32::layered_alpha(c.panel.hwnd))
    });
    wait(300);
    let seen = on(|c| {
        let p = win32::window_rect(c.panel.hwnd);
        let b = c.chats.bounds(&c.active()).unwrap_or_default();
        win32::Rect { x: p.x + b.x, y: p.y + b.y, w: b.w, h: b.h }
    });
    let (_, _) = screen_grab("50-opacity-70", seen, (0, 0, 0));
    let back = on(|c| {
        c.set_pref("opacity", json!(1.0));
        (c.panel_alpha, win32::layered_alpha(c.panel.hwnd))
    });
    log!("opacity 70%: alpha {alpha:?} | back to 100%: {back:?} (expect (179, Some(179)), (255, Some(255)))");

    // The panel activated again (Alt+Tab back, a file picker closing): the keyboard goes back into
    // the chat, not to the panel's own page.
    on(|c| c.set_pref("pinned", json!(true)));
    on(|c| c.test_popup());
    wait(900);
    let away = on(|c| win32::force_foreground(c.toastwin.hwnd));
    wait(400);
    let lost = view_js(first, "document.hasFocus()");
    let back_front = on(|c| win32::force_foreground(c.panel.hwnd));
    wait(600);
    log!(
        "focus after the panel is activated again: went away {away} (page focus {lost}) | back {back_front} | chat has focus {} | panel {:?} | in front {} (expect true, false, true, true, Open, PANEL)",
        view_js(first, "document.hasFocus()"),
        panel_state(),
        on(|c| c.snap())
    );
    on(|c| {
        c.toasts_dismiss_all();
        c.set_pref("pinned", json!(false));
    });
    ensure_open();

    // A site's own error page (HTTP 4xx) is shown, not ChatDock's "can't connect" screen.
    on(move |c| c.chats.navigate(first, "https://discord.com/api/v9/users/@me"));
    wait(4000);
    let (load, visible) = on(move |c| (c.load_state.get(first).copied().unwrap_or("?"), c.chats.is_visible(first)));
    log!("site error page: load state {load} | view visible {visible} (expect ready, true)");
    on(move |c| c.load_home(first));
    wait(2500);
    ensure_open();

    // Hard reload (Shift) works and the page comes back.
    on(move |c| c.reload_app(first, true));
    wait(3500);
    log!("hard reload: load state {} (expect ready)", on(move |c| c.load_state.get(first).copied().unwrap_or("?")));

    // Resize grip: the inner edge follows the real cursor (physical px, any monitor layout).
    ensure_open();
    let (p0, left) = on(|c| (win32::window_rect(c.panel.hwnd), c.on_left()));
    let was = win32::cursor_pos();
    let edge_x = if left { p0.right() - 3 } else { p0.x + 3 };
    let mid_y = p0.y + p0.h / 2;
    set_cursor(edge_x, mid_y);
    on(|c| c.resize_start());
    set_cursor(if left { edge_x + 80 } else { edge_x - 80 }, mid_y);
    on(|c| c.resize_to());
    let p1 = on(|c| win32::window_rect(c.panel.hwnd));
    set_cursor(edge_x, mid_y);
    on(|c| {
        c.resize_to();
        c.resize_grab = None;
    });
    let p2 = on(|c| win32::window_rect(c.panel.hwnd));
    set_cursor(was.0, was.1);
    log!("resize grip: width {} -> {} -> {} (expect +80, then back)", p0.w, p1.w, p2.w);
    close_panel();

    // "Clear data" of an app that isn't loaded (switched off / asleep) really clears it.
    let sleeper = on(|c| {
        let a = c.active();
        c.enabled_apps().into_iter().find(|id| *id != a).unwrap_or(c.enabled_apps()[0])
    });
    on(move |c| {
        c.set_app_pref(sleeper, "sleep", true);
        c.last_used.insert(sleeper.to_string(), 0);
        c.sleep_check();
    });
    wait(1500);
    let asleep = on(move |c| !c.chats.has(sleeper));
    on(move |c| c.clear_app_data(sleeper));
    wait(4000);
    log!("clear data of {sleeper} while asleep {asleep}: cleared {} (expect true, true)", log_has(&format!("data cleared {sleeper}")));
    on(move |c| c.set_app_pref(sleeper, "sleep", false));

    // Several monitors ("Automatic"): the edge only counts where the mouse really stops (the outer
    // edge of any monitor), the tab comes out on that monitor, and the hotkey opens the panel on
    // the monitor the mouse is on.
    let screens = win32::displays();
    if screens.len() > 1 {
        let hold_was = on(|c| {
            let h = c.settings.get("edgeHold").clone();
            c.settings.set("edgeHold", json!(0));
            h
        });
        for m in screens.clone() {
            let (x, y, outer) = on({
                let m = m.clone();
                move |c| {
                    let y = m.bounds.y + m.bounds.h / 2;
                    let x = if c.on_left() { m.bounds.x } else { m.bounds.right() - 1 };
                    (x, y, c.outer_edge_at(&m, y))
                }
            });
            on(move |c| {
                c.hide_tab(true);
                c.edge.test_cursor = Some((x, y, false));
            });
            wait(700);
            let (shown, tab) = on(|c| (c.edge.tab_shown, c.tab_bounds()));
            let on_it = shown && m.bounds.contains(tab.x + tab.w / 2, tab.y + tab.h / 2);
            log!("monitor {}: dock-side edge outer {outer} | tab out {shown} | on that monitor {on_it} (expect outer = tab out = on that monitor)", m.id);
            on(|c| {
                c.edge.test_cursor = None;
                c.hide_tab(true);
            });
            wait(300);
        }
        on(move |c| c.settings.set("edgeHold", hold_was));
        let was = win32::cursor_pos();
        for m in screens {
            set_cursor(m.bounds.x + m.bounds.w / 2, m.bounds.y + m.bounds.h / 2);
            on(|c| c.open_panel(None, "hotkey"));
            wait(700);
            let p = on(|c| win32::window_rect(c.panel.hwnd));
            log!("hotkey with the mouse on {}: panel there {} (expect true)", m.id, m.bounds.contains(p.x + p.w / 2, p.y + p.h / 2));
            close_panel();
        }
        set_cursor(was.0, was.1);
    }

    // "Clear data" of an app that is loaded: its page leaves first, then the profile is wiped and
    // the home page loads again.
    let loaded = on(|c| c.active());
    let loaded2 = loaded.clone();
    on(move |c| c.clear_app_data(&loaded2));
    wait(6000);
    let loaded3 = loaded.clone();
    let state = on(move |c| (c.load_state.get(loaded3.as_str()).copied().unwrap_or("?"), c.chats.source(&loaded3)));
    log!(
        "clear data of {loaded} (loaded): cleared {} | load state {} | page {} (expect true, ready, its home page)",
        log_has(&format!("data cleared {loaded}")),
        state.0,
        state.1
    );

    let m = on(|c| c.memory_stats());
    log!("RAM after the fix checks: {} MB in {} processes", m.total, m.processes);

    // What's new is written for where the user comes from.
    let lists = on(|c| {
        let now = rt::version();
        let mut out = Vec::new();
        for from in ["1.3.0", "1.5.0", "1.5.1"] {
            c.settings.set("whatsNew", json!({ "version": now, "from": from, "notes": "", "at": rt::epoch_ms() }));
            let versions: Vec<String> = c.whats_new_sections().into_iter().map(|(v, _)| v).collect();
            out.push(format!("from {from}: {}", versions.join(" ")));
        }
        out
    });
    log!("what's new {} (expect the hold (1.4.0) first, then newest first; 1.5.1 only from 1.5.0)", lists.join(" | "));
}

/// The What's new window after an update: what it lists for where the user came from, its size
/// and place, what it looks like on the real screen, and its buttons. Never takes the keyboard.
fn whats_new_test() {
    let now = rt::version();
    let cases = [
        ("the last version", previous_release()),
        ("1.5.0", "1.5.0".to_string()),
        ("1.3.0 (Electron)", "1.3.0".to_string()),
        ("older", "older".to_string()),
        ("notes only", now.clone()),
    ];
    for (i, (name, from)) in cases.into_iter().enumerate() {
        let (from2, now2) = (from.clone(), now.clone());
        let fg = win32::foreground_window();
        on(move |c| {
            let notes = "### New\n- **First** point of the notes\n- Second point with `code`\n\n### Install\nRun it.";
            c.settings.set("whatsNew", json!({ "version": now2, "from": from2, "notes": notes, "at": rt::epoch_ms() }));
            c.announce_updated();
        });
        wait(2500);
        let shot_name = format!("50-whats-new-{i}");
        shot(&shot_name);
        let (made, visible, rect, d) = on(|c| match c.whatsnew.win.as_ref() {
            Some(w) => {
                let r = win32::window_rect(w.hwnd);
                (true, win32::is_visible(w.hwnd), r, c.display_at(r.x + r.w / 2, r.y + r.h / 2).map(|d| (d.id, d.work, d.ui)))
            }
            None => (false, false, win32::Rect::default(), None),
        });
        let page = page_js(
            "whatsnew",
            "JSON.stringify({ title: document.getElementById('title').textContent, route: document.getElementById('route').textContent, \
             sections: [...document.querySelectorAll('.notes h3')].map(h => h.textContent), lines: document.querySelectorAll('.notes li').length, \
             first: document.querySelector('.notes li')?.textContent.slice(0, 50), scrolls: document.getElementById('notes').scrollHeight > document.getElementById('notes').clientHeight + 1, \
             lang: document.documentElement.lang, ok: document.getElementById('ok').textContent,              sizes: (() => { const c = document.querySelector('.card'), n = document.getElementById('notes');                const now = [innerWidth, innerHeight, devicePixelRatio, c.offsetHeight, n.clientHeight, n.scrollHeight, n.offsetWidth - n.clientWidth];                c.classList.add('measure'); const m = [c.offsetHeight, n.offsetHeight, n.scrollHeight]; c.classList.remove('measure'); return now.concat(m); })() })",
        );
        let centred = d.as_ref().is_some_and(|(_, work, _)| {
            ((rect.x + rect.w / 2) - (work.x + work.w / 2)).abs() <= 2 && ((rect.y + rect.h / 2) - (work.y + work.h / 2)).abs() <= 2
        });
        let now_fg = win32::foreground_window();
        let keyboard = if now_fg == fg {
            "kept".to_string()
        } else if on(move |c| c.whatsnew.win.as_ref().is_some_and(|w| w.hwnd == now_fg)) {
            "TAKEN by What's new".to_string()
        } else {
            format!("moved to {} (not us)", win32::process_name(now_fg))
        };
        log!(
            "what's new from {name} ({from}): made {made} | visible {visible} | {} x {} on {:?} | centred {centred} | keyboard {keyboard} | {page}",
            rect.w,
            rect.h,
            d.as_ref().map(|(id, _, ui)| format!("{id} ui {ui:.2}")),
        );
        let (blank, _) = screen_grab(&shot_name, rect, (24, 25, 34));
        log!("what's new on screen: card colour {blank}% of the window (expect most of it; 0 = covered or not drawn)");
        let button = if i == 1 { "#x" } else { "#ok" };
        let clicked = js_click("whatsnew", button);
        wait(700);
        log!("closed with {button} (clicked {clicked}): gone {} (expect true)", on(|c| c.whatsnew.win.is_none()));
    }
    // too much to fit: the list scrolls inside a window that stays on the screen
    let tall = on(|c| {
        let lines: Vec<String> = (1..=60).map(|n| format!("- Point number {n} of a very long list of changes")).collect();
        c.settings
            .set("whatsNew", json!({ "version": rt::version(), "from": rt::version(), "notes": lines.join("\n"), "at": rt::epoch_ms() }));
        c.announce_updated();
        c.whats_new_sections().first().map(|s| s.1.len()).unwrap_or(0)
    });
    wait(2500);
    let fits = page_js("whatsnew", "document.getElementById('notes').scrollHeight <= document.getElementById('notes').clientHeight + 1");
    // a list taller than the screen: the page asks for more than there is
    on(|c| c.whats_new_size(5000.0));
    wait(500);
    let (rect, work) = on(|c| {
        let r = c.whatsnew.win.as_ref().map(|w| win32::window_rect(w.hwnd)).unwrap_or_default();
        (r, c.display_at(r.x + r.w / 2, r.y + r.h / 2).map(|d| d.work).unwrap_or_default())
    });
    let clamped = rect.y >= work.y && rect.y + rect.h <= work.y + work.h && rect.h <= work.h * 85 / 100 + 1;
    // and a window too short for its list: it scrolls
    on(|c| c.whats_new_size(200.0));
    wait(500);
    log!(
        "long list ({tall} points, the notes keep 12): fits without scrolling {fits} | asked for 5000 px: {} x {} in a work area {} high, inside it and at most 85% {clamped} | asked for 200 px: scrolls {} (expect 12, true, true, true)",
        rect.w,
        rect.h,
        work.h,
        page_js("whatsnew", "document.getElementById('notes').scrollHeight > document.getElementById('notes').clientHeight")
    );
    shot("51-whats-new-long");
    // Esc (the page's own key handler; a real key press would need the keyboard)
    page_js("whatsnew", "document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); true");
    wait(700);
    log!("Esc: closed {} (expect true)", on(|c| c.whatsnew.win.is_none()));
}

/// The settings screen, page by page in both themes, a search, and the monitor page with monitor
/// layouts that aren't plugged in (page captures for looking at the design). The panel is shown off
/// screen and never activated: nothing takes the keyboard.
fn settings_shots() {
    on(|c| {
        c.settings_mode = true;
        c.help_mode = false;
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.broadcast_state();
    });
    wait(2000);
    let names = page_js("panel", "JSON.stringify([...document.querySelectorAll('.set-page')].map(p => p.dataset.page))");
    let pages: Vec<String> =
        serde_json::from_str::<String>(&names).ok().and_then(|list| serde_json::from_str(&list).ok()).unwrap_or_default();
    let theme_was = on(|c| c.settings.get("theme").clone());
    for theme in ["dark", "light"] {
        on(move |c| {
            c.settings.set("theme", json!(theme));
            c.apply_theme();
        });
        wait(800);
        for p in &pages {
            let p2 = p.clone();
            on(move |c| c.emit("panel", "settings:goto", json!([p2])));
            wait(700);
            shot(&format!("60-{theme}-{p}"));
        }
    }
    on(move |c| {
        c.settings.set("theme", json!("dark"));
        c.apply_theme();
    });
    log!(
        "settings pages: {} | page heights {}",
        pages.join(" "),
        page_js("panel", "JSON.stringify([...document.querySelectorAll('.set-page')].map(p => p.dataset.page + ':' + p.scrollHeight))")
    );

    // search, from the home page
    on(|c| c.emit("panel", "settings:goto", json!(["home"])));
    wait(600);
    for (i, word) in ["จอ", "pop", "zzqx"].iter().enumerate() {
        let found = page_js(
            "panel",
            &format!(
                "(() => {{ const s = document.querySelector('[data-search]'); s.value = {}; s.dispatchEvent(new Event('input')); \
                 return JSON.stringify([...document.querySelectorAll('.result b')].map(b => b.textContent).slice(0, 6).concat(document.querySelector('.no-results')?.textContent || [])); }})()",
                json!(word)
            ),
        );
        log!("search {word:?}: {found}");
        shot(&format!("61-search-{i}"));
    }
    // Esc steps back: first it clears the search, then it leaves a page for the home page (one more
    // would close Settings; not done here, that hands the keyboard to the panel)
    on(|c| c.on_key(0x1B, false, false, false, true));
    wait(600);
    let cleared =
        page_js("panel", "document.querySelector('[data-search]').value === '' && document.querySelector('[data-results]').hidden");
    on(|c| c.emit("panel", "settings:goto", json!(["apps"])));
    wait(600);
    on(|c| c.on_key(0x1B, false, false, false, true));
    wait(600);
    log!(
        "Esc: search cleared {cleared} | from a page back to the home page {} | Settings still open {} (expect true, true, true)",
        page_js("panel", "!document.querySelector('.set-page[data-page=home]').hidden"),
        on(|c| c.settings_mode)
    );

    // the monitor page with layouts that aren't plugged in
    use win32::Display as D;
    let two = vec![D::test(1, "DELL U2720Q", false, (0, 0, 2560, 1440), true), D::test(2, "", true, (2560, 360, 1920, 1080), false)];
    let three = vec![
        D::test(1, "LG ULTRAGEAR", false, (-1920, 0, 1920, 1080), false),
        D::test(2, "ASUS VG27AQ", false, (0, -180, 2560, 1440), true),
        D::test(3, "DELL P2419H", false, (2560, -420, 1080, 1920), false),
    ];
    let stacked = vec![D::test(1, "Odyssey G7", false, (0, -1440, 2560, 1440), false), D::test(2, "", true, (320, 0, 1920, 1080), true)];
    let middle = three[1].key.clone();
    let layouts: Vec<(&str, Vec<D>, Option<String>, &str)> = vec![
        ("two", two.clone(), None, "right"),
        ("two-pinned", two.clone(), Some(two[0].key.clone()), "right"),
        ("three", three.clone(), None, "right"),
        ("three-middle-seam", three.clone(), Some(middle), "right"),
        ("three-left", three.clone(), None, "left"),
        ("stacked", stacked, None, "right"),
        ("missing", two, Some(r"\\?\display#benq#gone".to_string()), "right"),
    ];
    let (id_was, label_was, side_was) =
        on(|c| (c.settings.get("displayId").clone(), c.settings.get("displayLabel").clone(), c.settings.get("side").clone()));
    for (name, list, pick, side) in layouts {
        on(move |c| {
            *win32::TEST_DISPLAYS.lock().unwrap() = Some(list);
            c.settings.set("displayId", pick.map(|k| json!(k)).unwrap_or(serde_json::Value::Null));
            c.settings.set("displayLabel", json!("BenQ EX2780Q"));
            c.settings.set("side", json!(side));
            c.dock_display.clear();
            c.on_displays_changed();
            c.emit("panel", "settings:goto", json!(["monitors"]));
        });
        wait(900);
        let state = on(|c| {
            let s = c.settings_state();
            json!({ "mode": s["monitorMode"], "seam": s["seam"],
                    "monitors": s["monitors"].as_array().map(|l| l.iter().map(|m| json!([m["n"], m["label"], m["chat"], m["edges"]])).collect::<Vec<_>>()) })
            .to_string()
        });
        let page = page_js(
            "panel",
            "JSON.stringify({ tiles: document.querySelectorAll('.mon').length, lit: document.querySelectorAll('.edge-mark').length, \
             chosen: document.querySelector('.mon.chosen .num')?.textContent || '', radios: [...document.querySelectorAll('.radio')].map(r => (r.classList.contains('on') ? '*' : '') + r.querySelector('b').textContent), \
             seam: !document.querySelector('[data-notice=seam]').hidden, caption: document.querySelector('.map-cap').textContent.slice(0, 60) })",
        );
        log!("monitors {name}: {state} | page {page}");
        shot(&format!("62-monitors-{name}"));
    }
    // pop-ups: on the chat's monitor, the main one, or where the mouse was when they started
    let popup_was = on(|c| c.settings.get("popupDisplay").clone());
    let popups = on(move |c| {
        *win32::TEST_DISPLAYS.lock().unwrap() =
            Some(vec![D::test(1, "DELL U2720Q", false, (0, 0, 2560, 1440), true), D::test(2, "", true, (2560, 360, 1920, 1080), false)]);
        c.settings.set("displayId", serde_json::Value::Null);
        c.dock_display.clear();
        let mut out = Vec::new();
        for (pick, mouse_on) in [("chat", ""), ("main", ""), ("mouse", r"\\.\TESTDISPLAY1"), ("mouse", "")] {
            c.settings.set("popupDisplay", json!(pick));
            c.toasts.display = mouse_on.to_string();
            out.push(format!("{pick}{}: {}", if mouse_on.is_empty() { "" } else { "(mouse on 1)" }, c.toast_display().id));
        }
        c.settings.set("popupDisplay", popup_was);
        c.toasts.display.clear();
        out.join(" | ")
    });
    log!("pop-ups show on: {popups} (expect TESTDISPLAY2 (the chat's, outermost right) | TESTDISPLAY1 (main) | TESTDISPLAY1 | TESTDISPLAY2 (unknown: the chat's))");

    // choosing from the page itself: a monitor in the map, then Automatic again
    on(move |c| {
        *win32::TEST_DISPLAYS.lock().unwrap() =
            Some(vec![D::test(1, "DELL U2720Q", false, (0, 0, 2560, 1440), true), D::test(2, "", true, (2560, 360, 1920, 1080), false)]);
        c.settings.set("displayId", serde_json::Value::Null);
        c.on_displays_changed();
    });
    wait(700);
    js_click("panel", ".mon[data-monitor*='test2']");
    wait(500);
    let picked =
        on(|c| (c.settings.get("displayId").clone(), c.settings.get("displayLabel").clone(), c.auto_display(), c.target_display().id));
    js_click("panel", ".radio[data-monitor='auto']");
    wait(500);
    log!(
        "picked in the map: {picked:?} | back to Automatic: {} (expect the test2 key + its label + false + TESTDISPLAY2, then null)",
        on(|c| c.settings.get("displayId").clone())
    );
    review_checks();
    identify_test();
    on(move |c| {
        *win32::TEST_DISPLAYS.lock().unwrap() = None;
        c.settings.set("displayId", id_was);
        c.settings.set("displayLabel", label_was);
        c.settings.set("side", side_was);
        c.settings.set("theme", theme_was);
        c.apply_theme();
        win32::forget_displays();
        c.on_displays_changed();
        c.settings_mode = false;
        win32::hide(c.panel.hwnd);
        c.broadcast_state();
    });
}

/// Things a code review caught in the new Settings, checked on two made-up monitors.
fn review_checks() {
    use win32::Display as D;
    let two = vec![D::test(1, "DELL U2720Q", false, (0, 0, 2560, 1440), true), D::test(2, "", true, (2560, 360, 1920, 1080), false)];
    let (k1, k2) = (two[0].key.clone(), two[1].key.clone());
    let list = two.clone();
    on(move |c| {
        *win32::TEST_DISPLAYS.lock().unwrap() = Some(list);
        c.settings.set("displayId", serde_json::Value::Null);
        c.dock_display.clear();
        c.on_displays_changed();
        c.emit("panel", "settings:goto", json!(["monitors"]));
    });
    wait(700);

    // "The chat opens here" only on the card of the chat's monitor (monitor 2: outermost right)
    on(|c| c.identify_all());
    wait(1800);
    let badges: Vec<String> = ["ident-testdisplay1", "ident-testdisplay2"]
        .iter()
        .map(|l| page_js(l, "getComputedStyle(document.getElementById('chat')).display"))
        .collect();
    log!("review: 'chat opens here' shown on the cards {badges:?} (expect [\"none\", \"flex\"])");

    // picking the monitor the mouse rests on keeps its card up
    let k1b = k1.clone();
    on(move |c| c.identify_hover(&k1b));
    wait(900);
    let k1c = k1.clone();
    on(move |c| {
        c.set_pref("displayId", json!(k1c));
    });
    wait(900);
    let card = |label: &'static str| {
        on(move |_| {
            tauri::Manager::get_webview_window(rt::app(), label)
                .and_then(|w| w.hwnd().ok())
                .is_some_and(|h| win32::is_visible(h.0 as isize))
        })
    };
    log!("review: after picking the hovered monitor its card is still up {} (expect true)", card("ident-testdisplay1"));
    on(|c| c.identify_hover(""));

    // a value saved by 1.5.x (Windows' name) ticks the right monitor in the list
    on(|c| c.settings.set("displayId", json!(r"\\.\TESTDISPLAY2")));
    let shown = on(|c| c.settings_state()["prefs"]["displayId"].as_str().unwrap_or("").to_string());
    log!("review: a 1.5.x value shows as monitor 2's key {} (expect true)", shown == k2);
    on(|c| {
        c.settings.set("displayId", serde_json::Value::Null);
        c.broadcast_state();
    });

    // a search result, Esc: back to the home page with the results, then one more clears them
    on(|c| c.emit("panel", "settings:goto", json!(["home"])));
    wait(500);
    page_js("panel", "(() => { const s = document.querySelector('[data-search]'); s.value = 'hotkey'; s.dispatchEvent(new Event('input')); document.querySelector('.result')?.click(); return 1; })()");
    wait(600);
    let jumped = page_js("panel", "document.querySelector('.set-page[data-page=home]').hidden");
    on(|c| c.on_key(0x1B, false, false, false, true));
    wait(600);
    let back =
        page_js("panel", "!document.querySelector('.set-page[data-page=home]').hidden && !document.querySelector('[data-results]').hidden");
    wait(300);
    on(|c| c.on_key(0x1B, false, false, false, true));
    wait(600);
    let cleared = page_js("panel", "document.querySelector('[data-search]').value === ''");
    log!("review: result opened its page {jumped} | Esc back to the results {back} | Esc again clears the search {cleared} (expect true, true, true)");

    // a result the keyboard is on keeps the focus through the updates ChatDock sends every few seconds
    page_js("panel", "(() => { const s = document.querySelector('[data-search]'); s.value = 'pop'; s.dispatchEvent(new Event('input')); document.querySelector('.result')?.focus(); return 1; })()");
    on(|c| c.broadcast_state());
    wait(500);
    log!(
        "review: focused result kept through a state update {} (expect true)",
        page_js("panel", "document.activeElement && document.activeElement.classList.contains('result')")
    );
    page_js(
        "panel",
        "(() => { const s = document.querySelector('[data-search]'); s.value = ''; s.dispatchEvent(new Event('input')); return 1; })()",
    );
    on(|c| c.identify_close());
}

/// "Show numbers on the screens" on the real monitors: every card for a moment, then the one the
/// mouse rests on in the map. Real screen grabs; the cards never take the keyboard.
fn identify_test() {
    on(|c| {
        *win32::TEST_DISPLAYS.lock().unwrap() = None;
        win32::forget_displays();
        c.on_displays_changed();
    });
    wait(500);
    let fg = win32::foreground_window();
    on(|c| c.identify_all());
    wait(1500);
    let cards = on(|c| {
        c.numbered_displays()
            .into_iter()
            .map(|(n, d)| {
                let label =
                    format!("ident-{}", d.id.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase());
                let w = tauri::Manager::get_webview_window(rt::app(), &label);
                let hwnd = w.as_ref().and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize).unwrap_or(0);
                (n, d.bounds, label, hwnd, win32::is_visible(hwnd), win32::window_rect(hwnd))
            })
            .collect::<Vec<_>>()
    });
    for (n, b, label, _, visible, r) in &cards {
        let centred = ((r.x + r.w / 2) - (b.x + b.w / 2)).abs() <= 2 && ((r.y + r.h / 2) - (b.y + b.h / 2)).abs() <= 2;
        let page = page_js(label, "JSON.stringify([document.getElementById('num').textContent, document.getElementById('name').textContent, document.getElementById('detail').textContent, !document.getElementById('chat').hidden])");
        let (dark, _) = screen_grab(&format!("63-identify-{n}"), *r, (22, 23, 31));
        log!("identify: monitor {n} card visible {visible} | centred {centred} | {page} | on screen: card colour {dark}%");
    }
    let now_fg = win32::foreground_window();
    log!(
        "identify: keyboard kept {} (expect true) | before {} {} | now {} {}",
        now_fg == fg,
        win32::process_name(fg),
        win32::class_name(fg),
        win32::process_name(now_fg),
        win32::class_name(now_fg)
    );
    wait(2000);
    log!(
        "identify: after 3.5 s all hidden {} (expect true)",
        on(|c| c.numbered_displays().iter().all(|(_, d)| {
            let label = format!("ident-{}", d.id.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase());
            tauri::Manager::get_webview_window(rt::app(), &label)
                .and_then(|w| w.hwnd().ok())
                .is_none_or(|h| !win32::is_visible(h.0 as isize))
        }))
    );
    let key = on(|c| c.numbered_displays().first().map(|(_, d)| d.key.clone()).unwrap_or_default());
    let key2 = key.clone();
    on(move |c| c.identify_hover(&key2));
    wait(600);
    let shown = cards.first().is_some_and(|(_, _, _, h, _, _)| win32::is_visible(*h));
    on(|c| c.identify_hover(""));
    wait(300);
    let hidden = cards.first().is_some_and(|(_, _, _, h, _, _)| !win32::is_visible(*h));
    log!("identify: hover over monitor 1 in the map shows its card {shown}, leaving hides it {hidden} (expect true, true)");
    on(|c| c.identify_close());
}

/// Unread numbers = what the site counts minus what was seen while its chat was on screen. Replays
/// what happened on Instagram (a message arrives in the conversation being read, the title shows
/// "(1)" and clears at once, the chat is closed in that moment: that must not pop up), then a new
/// message while away, a site that counts other things too, and a new page (a reload, a wake, a
/// restart). The panel isn't really opened (nothing takes the keyboard): "on screen" is set directly.
fn counts_test() {
    let app = on(|c| c.enabled_apps()[0]);
    let count = move || on(move |c| c.counts.get(app).copied().unwrap_or(0));
    let (popups_were, active_was) = on(|c| (c.settings.get("popups").clone(), c.active()));
    on(move |c| {
        c.toasts.offscreen = true; // (pop-ups shown off the screen: the user may be at the PC)
        c.settings.set("popups", json!(true));
        c.settings.set_in("seenCounts", app, json!(0));
        c.load_started_at.insert(app.to_string(), 0);
        c.last_content_at.insert(app.to_string(), 0);
        c.last_flash.remove(app);
        c.set_setting("active", json!(app));
        c.on_title(app, "Instagram");
    });
    wait(3300);
    on(move |c| c.on_title(app, "(2) Instagram"));
    let hidden = count();
    on(|c| {
        c.panel_state = PanelState::Open;
        c.counts_seen_in_view();
    });
    let in_view = count();
    on(move |c| c.on_title(app, "(3) Instagram"));
    let while_reading = count();
    on(move |c| c.on_title(app, "Instagram"));
    wait(400);
    on(|c| c.panel_state = PanelState::Hidden); // closed right after replying
    wait(3400);
    let (after_close, popups_after_close) = (count(), on(|c| c.toasts_count()));
    on(move |c| {
        c.last_content_at.insert(app.to_string(), 0);
        c.on_title(app, "(1) Instagram");
    });
    wait(2900);
    let (new_msg, popups_new) = (count(), on(|c| c.toasts_count()));
    on(|c| c.toasts_dismiss_all());
    on(move |c| c.on_title(app, "(5) Instagram"));
    let five = count();
    on(|c| {
        c.panel_state = PanelState::Open;
        c.counts_seen_in_view();
    });
    let opened = count();
    on(|c| c.panel_state = PanelState::Hidden);
    on(move |c| c.on_title(app, "(6) Instagram"));
    let one_more = count();
    let kept = on(move |c| c.settings.get("seenCounts").get(app).cloned());
    on(move |c| c.on_title(app, "(1) Instagram"));
    let dropped = count();
    log!(
        "counts ({app}): hidden {hidden} | on screen {in_view} | message while reading {while_reading} | closed right away: number {after_close}, pop-ups {popups_after_close} | new message while away: number {new_msg}, pop-ups {popups_new} | site counts 5 while away: {five}, opened: {opened}, a 6th: {one_more}, seen kept {kept:?} | site drops to 1: {dropped} (expect 2 | 0 | 0 | 0, 0 | 1, 1 | 5, 0, 1, Some(5) | 0)"
    );
    // A new page: while it loads without a number the remembered "seen" stays; the same number
    // again is still seen; a lower one went down while nobody watched, so all of it shows.
    on(move |c| {
        c.counted_pages.remove(app); // what the page's ContentLoading does
        c.on_title(app, "Instagram");
    });
    wait(3300);
    let seen_while_loading = on(move |c| c.settings.get("seenCounts").get(app).cloned());
    on(move |c| c.on_title(app, "(1) Instagram"));
    let same_again = count();
    on(move |c| {
        c.counted_pages.remove(app); // a restart
        c.load_started_at.insert(app.to_string(), rt::epoch_ms()); // (a page that just loaded: no count pop-up)
        c.settings.set_in("seenCounts", app, json!(5));
        c.on_title(app, "(5) Instagram");
    });
    let restart_same = count();
    on(move |c| {
        c.counted_pages.remove(app);
        c.on_title(app, "(2) Instagram");
    });
    let restart_lower = count();
    log!(
        "counts ({app}) on a new page: seen while it loads {seen_while_loading:?}, the same number again {same_again} | after a restart: the same number {restart_same}, a lower one {restart_lower} (expect Some(1), 0 | 0, 2)"
    );
    // Discord without a notification of its own yet (its desktop notifications off): its count
    // pop-up says where to turn them on
    let dc = on(|c| c.is_enabled("discord"));
    let dc_hint = if dc {
        on(|c| {
            c.last_content_at.insert("discord".into(), 0);
            c.load_started_at.insert("discord".into(), 0);
            c.on_title("discord", "(1) Discord");
        });
        wait(2900);
        shot("counts-discord-hint"); // (the hint wraps: the card grows, nothing is cut off)
        page_js("toasts", "[...document.querySelectorAll('.card:not(.leaving) .hint')].map((h) => h.textContent).join(' | ')")
    } else {
        "(Discord is off)".into()
    };
    let dc_text = on(|c| c.t("toast.discordWho"));
    log!("counts: Discord's count pop-up with no notification from it: {dc_hint} (expect \"{dc_text}\")");
    on(move |c| {
        c.toasts_dismiss_all();
        c.on_title(app, "Instagram");
        if dc {
            c.on_title("discord", "Discord");
        }
        c.settings.set("popups", popups_were);
        c.set_setting("active", json!(active_was));
    });
    wait(3300);
    on(|c| c.toasts.offscreen = false);
}

/// Discord extras: where a notification is from, per-server pop-ups, and the voice keys (a real
/// global hotkey through Windows, pressing a stand-in for Discord's mute switch in the page).
/// Never takes the keyboard.
fn discord_test() {
    wait(7000); // the pages load
    let samples = ["Alice (#general, My Server)", "Bob (#off-topic, Gamers, Inc.)", "Carol", "Dave (Weekend squad)", "Eve (#, X)"];
    let parsed: Vec<String> = samples.iter().map(|t| format!("{t:?} -> {:?}", crate::core::discord_place(t))).collect();
    log!("discord titles: {}", parsed.join(" | "));

    // a server message: a pop-up saying where, the server listed (on)
    let popups_were = on(|c| c.settings.get("popups").clone());
    on(|c| {
        c.settings.set("popups", json!(true));
        c.settings.set("discordServers", json!({}));
        c.toasts_dismiss_all();
        c.on_site_notification("discord", 0, "Alice (#general, My Server)", "gg", "", "t1");
    });
    wait(900);
    let card = page_js(
        "toasts",
        "JSON.stringify([document.querySelector('.card .meta')?.textContent, document.querySelector('.card .title')?.textContent])",
    );
    let listed = on(|c| c.settings.get("discordServers").clone());
    // that server switched off: nothing pops up; a direct message still does
    on(|c| {
        c.toasts_dismiss_all();
        c.settings_action_test("discord-server", json!({ "name": "My Server", "on": false }));
        c.on_site_notification("discord", 0, "Alice (#general, My Server)", "gg again", "", "t2");
    });
    wait(700);
    let off = on(|c| c.toasts_count());
    on(|c| c.on_site_notification("discord", 0, "Carol", "hi", "", "t3"));
    wait(700);
    let dm = on(|c| c.toasts_count());
    log!("discord pop-ups: card {card} | servers {listed} | server switched off: pop-ups {off} | a direct message: pop-ups {dm} (expect [\"Discord · My Server · #general\",\"Alice\"], {{\"My Server\":true}}, 0, 1)");
    on(move |c| {
        c.toasts_dismiss_all();
        c.settings.set("popups", popups_were);
    });

    // voice keys: a stand-in for Discord's user panel (its mic and sound switches), then the real
    // global hotkey through Windows
    view_js(
        "discord",
        "(() => { const s = document.createElement('section'); s.className = 'panels_test'; \
         for (let i = 0; i < 4; i++) { const b = document.createElement('button'); b.setAttribute('role', 'switch'); b.setAttribute('aria-checked', 'false'); \
         b.onclick = () => b.setAttribute('aria-checked', b.getAttribute('aria-checked') === 'true' ? 'false' : 'true'); s.append(b); } \
         document.body.append(s); return 1; })()",
    );
    let registered = on(|c| {
        c.set_pref("discordMuteKey", json!("Control+Alt+Shift+M"));
        c.hotkeys.iter().any(|(_, a)| *a == "discord-mute")
    });
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() },
        },
    };
    let m = VIRTUAL_KEY(0x4D);
    let seq = [
        key(VK_CONTROL, false),
        key(VK_MENU, false),
        key(VK_SHIFT, false),
        key(m, false),
        key(m, true),
        key(VK_SHIFT, true),
        key(VK_MENU, true),
        key(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&seq, std::mem::size_of::<INPUT>() as i32) };
    wait(1200);
    let state = view_js("discord", "JSON.stringify([...document.querySelectorAll('section[class*=\"panels\"] button[role=\"switch\"]')].map(b => b.getAttribute('aria-checked')))");
    let said = page_js("toasts", "document.querySelector('.card .title')?.textContent || ''");
    shot("80-voice-key");
    log!("discord voice key: registered {registered} | sent {sent} | switches now {state} | pop-up says {said} (expect true, 8, [\"false\",\"false\",\"true\",\"false\"], the mic is off)");
    on(|c| {
        c.set_pref("discordMuteKey", json!(""));
        c.toasts_dismiss_all();
        c.on_site_notification("discord", 0, "Bob (#off-topic, Gamers, Inc.)", "hey", "", "t4"); // a second server for the page
        c.toasts_dismiss_all();
    });

    // the settings page (dark and light), off screen and never activated
    on(|c| {
        c.settings_mode = true;
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.broadcast_state();
        c.emit("panel", "settings:goto", json!(["discord"]));
    });
    wait(1500);
    for theme in ["dark", "light"] {
        on(move |c| {
            c.settings.set("theme", json!(theme));
            c.apply_theme();
        });
        wait(700);
        shot(&format!("81-discord-{theme}"));
    }
    log!(
        "discord page: {}",
        page_js("panel", "JSON.stringify({ shown: !document.querySelector('.set-page[data-page=discord]').hidden, servers: [...document.querySelectorAll('.dc-server b')].map(b => b.textContent), keys: document.querySelectorAll('[data-voice-keys] option').length })")
    );
    on(|c| {
        c.settings.set("theme", json!("system"));
        c.apply_theme();
        c.settings_mode = false;
        win32::hide(c.panel.hwnd);
        c.broadcast_state();
    });

    call_test();
}

/// A call in Discord's page (a real WebRTC call: no mic, camera or screen). It keeps the app awake
/// and its memory up, and the edge tab shows its icons. Nothing takes the focus or the keyboard.
fn call_test() {
    // A call keeps Discord awake and its memory up, even when nobody talks: a real WebRTC call in
    // the page (two connections talking to each other; no mic, camera or screen)
    let (active_was, sleep_was) = on(|c| {
        let was = (c.active(), c.settings.app_pref("discord", "sleep"));
        c.set_active("instagram", false); // Discord out of sight and not the app in use
        was
    });
    let start_call = "(() => { const a = new RTCPeerConnection(), b = new RTCPeerConnection(); \
         a.onicecandidate = (e) => e.candidate && b.addIceCandidate(e.candidate); \
         b.onicecandidate = (e) => e.candidate && a.addIceCandidate(e.candidate); \
         a.addTransceiver('audio'); \
         (async () => { await a.setLocalDescription(); await b.setRemoteDescription(a.localDescription); \
         await b.setLocalDescription(); await a.setRemoteDescription(b.localDescription); })(); \
         window.__testCall = [a, b]; return 1; })()";
    let wait_call = |want: bool| {
        for _ in 0..40 {
            if on(|c| c.chats.in_call("discord")) == want {
                return true;
            }
            wait(250);
        }
        false
    };
    let low_before = on(|c| c.chats.memory_low("discord"));
    view_js("discord", start_call);
    let started = wait_call(true);
    let states = view_js("discord", "JSON.stringify(window.__testCall.map((p) => p.connectionState))");

    // The icons on the edge tab: a phone while in a call, a screen while sharing (the share comes
    // from the page's own message here: a real one needs the screen picker). The tab is shown off
    // screen and never activated.
    let calls_in_call = on(|c| c.ui_state()["calls"].to_string());
    view_js("discord", "window.chrome.webview.postMessage(JSON.stringify({ type: 'call', live: 3, call: true, share: true })), 1");
    wait(400);
    let calls_sharing = on(|c| c.ui_state()["calls"].to_string());
    let tab_h = on(|c| {
        let d = c.target_display();
        let (w, h) = c.tab_size(&d);
        win32::set_bounds(c.tab.hwnd, win32::Rect { x: -30000, y: d.bounds.y + 100, w, h });
        c.emit("tab", "tab:show", json!([c.ui_state()]));
        win32::show_inactive(c.tab.hwnd);
        (h, ((74.0 + 40.0 * c.enabled_apps().len() as f64 + 90.0) * d.ui).round() as i32)
    });
    wait(700);
    shot("82-tab-call");
    let chips = page_js(
        "tab",
        "JSON.stringify([...document.querySelectorAll('.chip')].map((b) => b.className + ' ' + b.dataset.app + ' | ' + b.title))",
    );
    let fits = page_js("tab", "(() => { const p = document.getElementById('pill').getBoundingClientRect(), c = document.getElementById('calls').getBoundingClientRect(); return c.bottom <= p.bottom - 4 && c.top > p.top; })()");
    on(|c| {
        c.emit("tab", "tab:hide", json!([true]));
        win32::hide(c.tab.hwnd);
    });
    view_js("discord", "window.chrome.webview.postMessage(JSON.stringify({ type: 'call', live: 2, call: true, share: false })), 1");
    wait(400);
    let calls_after_share = on(|c| c.ui_state()["calls"].to_string());
    log!(
        "discord call icons: in a call {calls_in_call} | sharing too {calls_sharing} | tab {tab_h:?} high, chips {chips}, inside the pill {fits} | share stopped {calls_after_share} (expect [call], [call, share], equal heights, 2 chips, true, [call])"
    );
    let during = on(|c| {
        c.set_app_pref("discord", "sleep", true);
        c.last_used.insert("discord".into(), 0);
        c.sleep_check();
        (c.chats.memory_low("discord"), c.asleep.get("discord").copied().unwrap_or(false))
    });
    view_js("discord", "window.__testCall.forEach((p) => p.close()), 1");
    let hung_up = wait_call(false);
    let calls_end = on(|c| c.ui_state()["calls"].to_string());
    let low_after = on(|c| c.chats.memory_low("discord"));
    // a call, then the page reloads: the call ended with the old page
    view_js("discord", start_call);
    let again = wait_call(true);
    view_js("discord", "location.reload(), 1");
    let reloaded = wait_call(false);
    let slept = on(|c| {
        c.last_used.insert("discord".into(), 0);
        c.sleep_check();
        c.asleep.get("discord").copied().unwrap_or(false)
    });
    on(move |c| {
        c.set_app_pref("discord", "sleep", sleep_was); // (wakes it)
        c.set_active(&active_was, false);
    });
    log!(
        "discord call: memory low before {low_before} | call seen {started} {states} | during: memory low, asleep {during:?} | hung up {hung_up} (icons {calls_end}), memory low {low_after} | again {again}, after a reload {reloaded} | then asleep {slept} (expect true, true [connected x2], (false, false), true ([]), true, true, true, true)"
    );
}

/// Screen sharing puts up the "… is sharing your screen" bar: a window of the chats' browser that
/// takes the focus from the panel. A click elsewhere after that must still hide the chat (the
/// panel gets no signal of its own then), while our own dialogs keep it open as before. A pretend
/// foreground window: nothing takes the focus.
fn share_bar_test() {
    use windows::{
        core::{BOOL, PCWSTR},
        Win32::{
            Foundation::{HWND, LPARAM, TRUE},
            UI::WindowsAndMessaging::{EnumWindows, FindWindowW},
        },
    };
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        let list = &mut *(l.0 as *mut Vec<isize>);
        list.push(h.0 as isize);
        TRUE
    }
    let mut all: Vec<isize> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut Vec<isize> as isize));
    }
    let browser = crate::chats::browser_pid();
    let bar = on(move |c| all.into_iter().find(|&h| win32::window_pid(h) == browser && !c.is_own_window(h)).unwrap_or(0));
    let elsewhere = unsafe { FindWindowW(windows::core::w!("Shell_TrayWnd"), PCWSTR::null()) }.map(|h| h.0 as isize).unwrap_or(0);
    let toast = on(|c| c.toastwin.hwnd);
    let panel = on(|c| c.panel.hwnd);
    // the panel out (off screen, never activated)
    let open = || {
        on(|c| {
            c.test_foreground = None;
            c.focus_watch = false;
            let d = c.target_display();
            let g = c.panel_geometry(&d);
            win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
            win32::show_inactive(c.panel.hwnd);
            c.panel_state = PanelState::Open;
        })
    };
    let state = || on(|c| c.panel_state.as_str());
    let pretend = |h: isize| on(move |c| c.test_foreground = Some(h));

    // 1. the bar takes the focus: the chat stays and watches; the focus moves on to the taskbar: it hides
    open();
    pretend(bar);
    let watching = on(|c| {
        c.check_auto_hide(false);
        c.focus_watch
    });
    wait(500);
    let still = state();
    pretend(elsewhere);
    wait(700);
    let after = state();
    wait(500);
    // 2. the bar, then back into the chat: it stays, and stops watching
    open();
    pretend(bar);
    on(|c| c.check_auto_hide(false));
    wait(300);
    pretend(panel);
    wait(400);
    let back = on(|c| (c.panel_state.as_str(), c.focus_watch));
    // 3. one of our own windows had the focus (like our file picker), then the taskbar: as before,
    //    the chat stays
    pretend(toast);
    on(|c| c.check_auto_hide(false));
    let watching_own = on(|c| c.focus_watch);
    pretend(elsewhere);
    wait(600);
    let own_after = state();
    on(|c| {
        c.test_foreground = None;
        c.focus_watch = false;
        c.close_panel(false, "selftest");
    });
    wait(600);
    log!(
        "share bar: a window of the chats' browser {} ({}) | watching {watching}, after 0.5 s {still} | the focus moves to the taskbar: {after} | back into the chat: {back:?} | our own window had it: watching {watching_own}, then the taskbar: {own_after} (expect true, true, open, hidden, (\"open\", false), false, open)",
        bar != 0,
        win32::class_name(bar)
    );
}

/// Messenger and Instagram calls open a window of their own (made by WebView2, where the page
/// script doesn't run). A new window of the chats' browser after a call link counts as a call until
/// it closes, and a "… is sharing your screen" bar of its site as a screen share. Stand-ins: windows
/// of ChatDock itself, off screen, never activated.
fn call_window_test() {
    use windows::{
        core::{w, HSTRING},
        Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, ShowWindow, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
        },
    };
    let make = |title: &'static str, w: i32, h: i32| {
        on(move |_| unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                &HSTRING::from(title),
                WS_POPUP,
                -30000,
                -30000,
                w,
                h,
                None,
                None,
                None,
                None,
            )
            .map(|hwnd| {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                hwnd.0 as isize
            })
            .unwrap_or(0)
        })
    };
    let destroy = |hwnd: isize| {
        on(move |_| unsafe {
            let _ = DestroyWindow(win32::h(hwnd));
        })
    };
    let icons = || on(|c| c.ui_state()["calls"].to_string());
    on(|c| c.test_browser_pid = Some(std::process::id()));
    // a call link: its window appears a moment later
    on(|c| c.call_window_opening("instagram"));
    wait(300);
    let call_win = make("Instagram call", 480, 360);
    wait(900);
    let open = (icons(), on(|c| c.chats.in_call("instagram")));
    // the screen is shared from it
    let bar = make("www.instagram.com is sharing your screen.", 520, 44);
    wait(1500);
    let sharing = icons();
    destroy(bar);
    wait(1500);
    let share_stopped = icons();
    destroy(call_win);
    wait(1500);
    let closed = (icons(), on(|c| c.chats.in_call("instagram")));
    // a call link whose window never shows up
    on(|c| c.call_window_opening("facebook"));
    wait(8700);
    let gave_up = on(|c| c.call_window_search.is_none() && c.call_windows.is_empty());
    on(|c| c.test_browser_pid = None);
    log!(
        "call windows: open {open:?} | sharing {sharing} | share stopped {share_stopped} | closed {closed:?} | none showed up, gave up {gave_up} (expect ([instagram call], true), [call, share], [call], ([], false), true)"
    );
}

/// The What's new window's demos: one picture per scene, a moment into it. The window is off
/// screen and never activated.
fn news_demo_test() {
    // Settings → Updates and its "See what's new" button, pressed (the panel off screen, never
    // activated)
    on(|c| {
        c.whatsnew.offscreen = true;
        c.settings_mode = true;
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.broadcast_state();
        c.emit("panel", "settings:goto", json!(["updates"]));
    });
    wait(1500);
    shot("84-updates-page");
    let button = page_js(
        "panel",
        "(() => { const b = document.querySelector('[data-action=\"whats-new\"]'); if (!b || b.hidden || !b.offsetWidth) return 'not shown'; b.click(); return 'pressed: ' + b.textContent; })()",
    );
    wait(2500);
    log!("what's new from Settings: {button} | the window came {}", on(|c| c.whatsnew.win.is_some()));
    on(|c| {
        c.settings_mode = false;
        win32::hide(c.panel.hwnd);
        c.broadcast_state();
    });
    log!(
        "what's new demos: {}",
        page_js("whatsnew", "JSON.stringify({ title: document.getElementById('title').textContent, demos: document.querySelectorAll('#dots .dot').length, lines: document.querySelectorAll('.notes li').length, linked: document.querySelectorAll('.notes li[data-demo]').length, card: document.querySelector('.card').offsetHeight, window: innerHeight, scene: document.getElementById('scene').className })")
    );
    let n: usize = page_js("whatsnew", "document.querySelectorAll('#dots .dot').length").trim_matches('"').parse().unwrap_or(0);
    for i in 0..n {
        page_js("whatsnew", &format!("document.querySelectorAll('#dots .dot')[{i}].click(), 1"));
        for (at, ms) in [("a", 1200), ("b", 1400), ("c", 2000)] {
            wait(ms); // 1.2 s, 2.6 s and 4.6 s into the scene
            shot(&format!("83-news-demo-{i}{at}"));
        }
    }
    log!("what's new demo lit line: {}", page_js("whatsnew", "document.querySelector('.notes li.now')?.dataset.demo || ''"));
    on(|c| {
        c.close_whats_new(false);
        c.whatsnew.offscreen = false;
    });
}

/// Pictures of the What's new demos for the release notes: every scene, one picture per 80 ms of
/// its 6 s. The animations are stopped at each moment (not timed), so the frames come out even.
/// The window as it opens after an update, off screen. <shots>/gif/f<scene>-<ms>.png
fn news_gif_frames() {
    let Some(dir) = on(|c| c.args.shots.clone()) else { return };
    let dir = dir.join("gif");
    let _ = std::fs::create_dir_all(&dir);
    on(|c| {
        c.whatsnew.offscreen = true;
        c.show_whats_new_window();
    });
    wait(3000);
    let n: usize = page_js("whatsnew", "document.querySelectorAll('#dots .dot').length").trim_matches('"').parse().unwrap_or(0);
    let mut saved = 0;
    for i in 0..n {
        page_js("whatsnew", &format!("showDemo({i}, true), clearTimeout(demoTimer), 1"));
        wait(500); // the dot and the line light up
        for t in (0..6000).step_by(80) {
            page_js(
                "whatsnew",
                &format!(
                    "(() => {{ clearTimeout(demoTimer); const a = document.getAnimations().filter((x) => x.effect && x.effect.target && scene.contains(x.effect.target)); \
                     a.forEach((x) => {{ x.pause(); x.currentTime = {t}; }}); return a.length; }})()"
                ),
            );
            wait(35);
            let Some(w) = rt::app().get_webview_window("whatsnew") else { return };
            let (tx, rx) = mpsc::channel::<String>();
            let path = dir.join(format!("f{i}-{t:04}.png"));
            let _ = w.with_webview(move |pw| unsafe {
                if let Ok(wv) = pw.controller().CoreWebView2() {
                    capture(&wv, path, tx);
                }
            });
            if rx.recv_timeout(Duration::from_secs(5)).is_ok_and(|r| r == "ok") {
                saved += 1;
            }
        }
    }
    log!("gif frames: {n} scenes, {saved} pictures");
    on(|c| {
        c.close_whats_new(false);
        c.whatsnew.offscreen = false;
    });
}

// ---------------------------------------------------------------------------------------------
// The guide (1.7.4)
// ---------------------------------------------------------------------------------------------
/// Each step's scene runs this long (ms), and is checked and pictured at this moment.
const TOUR_STEPS: [(&str, u32, u32); 5] =
    [("edge", 8000, 7000), ("hotkey", 8000, 3200), ("popup", 8500, 7200), ("game", 9000, 7400), ("ready", 8000, 7000)];

/// The guide in the panel, shown off screen and never activated (nothing takes the keyboard): the
/// first start, or "How to use".
fn tour_show(onboarded: bool) {
    on(move |c| {
        c.settings.set("onboarded", json!(onboarded));
        c.settings_mode = false;
        c.help_mode = true;
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.broadcast_state();
    });
}

/// Every animation of the scene on the guide's stage stopped at `ms` into it (not timed).
fn tour_freeze(ms: u32) -> String {
    page_js(
        "panel",
        &format!(
            "(() => {{ const a = document.getElementById('tour-stage').getAnimations({{ subtree: true }}); a.forEach((x) => {{ x.pause(); x.currentTime = {ms}; }}); return a.length; }})()"
        ),
    )
}

/// What doesn't fit in the guide as it is now: a step's text wider than the panel, a label in the
/// scene cut short, the "stays on this PC" label over the chat, the game's menu off the stage, the
/// panel scrolling sideways. "[]" when all is well.
const TOUR_FIT: &str = "(() => { const out = []; const st = document.getElementById('tour-stage'); const box = st.getBoundingClientRect(); \
  for (const el of document.querySelectorAll('.tour-text')) if (el.scrollWidth > el.clientWidth + 1) out.push('text wider than the panel: ' + el.querySelector('h2').textContent.slice(0, 24)); \
  for (const el of st.querySelectorAll('.tx > *, .dd .v, .list p, .btn, .ch b, kbd, .mr span, .mt b')) if (el.scrollWidth > el.clientWidth + 1) out.push('cut: ' + el.textContent.slice(0, 24)); \
  const safe = st.querySelector('.safe .uf'), pnl = st.querySelector('.pnl'); \
  if (safe && pnl) { const a = safe.getBoundingClientRect(), b = pnl.getBoundingClientRect(); if (a.right > b.left + 1 && a.left < b.right - 1) out.push('the safe label covers the chat'); if (a.left < box.left - 1 || a.right > box.right + 1) out.push('the safe label is off the stage'); } \
  const menu = st.querySelector('.menu'); if (menu) { const r = menu.getBoundingClientRect(); if (r.left < box.left - 1 || r.right > box.right + 1) out.push('the menu is off the stage'); } \
  const page = document.getElementById('stage'); if (page.scrollWidth > page.clientWidth + 1) out.push('the panel scrolls sideways'); \
  return JSON.stringify(out); })()";

/// The clicks in the scenes: step, the moment of the press (ms), its ripple.
const TOUR_CLICKS: [(usize, u32, &str); 7] =
    [(0, 5120, "r1"), (2, 3060, "r1"), (3, 1755, "r1"), (3, 2745, "r2"), (4, 1080, "r1"), (4, 2200, "r2"), (4, 3640, "r3")];

/// How far the pointer's tip is from each click's ripple at the moment of the press, in px on the
/// page: "0,0" everywhere when the pointer clicks where the click shows.
fn tour_pointer() -> String {
    let mut out = Vec::new();
    for (step, ms, rip) in TOUR_CLICKS {
        page_js("panel", &format!("tourGo({step}), 1"));
        wait(250);
        tour_freeze(ms);
        wait(80);
        let off = page_js(
            "panel",
            &format!(
                "(() => {{ const st = document.getElementById('tour-stage'); const p = st.querySelector('.cur svg').getBoundingClientRect(); \
                 const r = st.querySelector('.rip.{rip}').getBoundingClientRect(); const k = p.width / 18; \
                 return Math.round(p.left + 2.25 * k - (r.left + r.width / 2)) + ',' + Math.round(p.top + 1.7 * k - (r.top + r.height / 2)); }})()"
            ),
        );
        out.push(format!("{step}@{ms}: {}", off.trim_matches('"')));
    }
    out.join(" | ")
}

/// A DevTools call in one of ChatDock's own pages (the self-test only: emulating "reduce motion").
fn page_cdp(label: &str, method: &str, params: &str) -> String {
    let Some(w) = rt::app().get_webview_window(label) else { return "err:no such window".into() };
    let (tx, rx) = mpsc::channel::<String>();
    let (method, params) = (method.to_string(), params.to_string());
    let _ = w.with_webview(move |pw| unsafe {
        if let Ok(wv) = pw.controller().CoreWebView2() {
            let done = tx.clone();
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |r, json| {
                let _ = done.send(if r.is_ok() { json } else { format!("err:{r:?}") });
                Ok(())
            }));
            if wv.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params), &handler).is_err() {
                let _ = tx.send("err:call".into());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| "err:timeout".into())
}

/// Sets the panel's width (DIP) where it stands, off screen.
fn panel_width(width: f64) {
    on(move |c| {
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        let w = (width * d.ui).round() as i32;
        let r = win32::Rect { w, ..g };
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&r, &d), ..r });
    });
}

fn tour_test() {
    let keys = ["onboarded", "theme", "side", "lang", "popups", "glow", "active"];
    let was: Vec<(&str, serde_json::Value)> = on(move |c| keys.iter().map(|k| (*k, c.settings.get(k).clone())).collect());
    on(|c| c.set_pref("lang", json!("en")));
    tour_show(false);
    wait(1800);
    log!(
        "guide on the first start: {} (expect shown, step 0, 5 texts, 5 dots, s-edge, Next, Skip the guide, Back hidden, no start box, k ~1)",
        page_js(
            "panel",
            "JSON.stringify({ shown: !document.getElementById('welcome').hidden, at: tourAt(), texts: document.querySelectorAll('.tour-text').length, \
             dots: document.querySelectorAll('.tour-dot').length, scene: document.getElementById('tour-stage').className, next: document.querySelector('#tour-next .label').textContent, \
             skip: document.getElementById('tour-skip').textContent, back: document.getElementById('tour-back').hidden, \
             box: ((r) => !r.hidden && !r.classList.contains('away'))(document.getElementById('autostart-row')), \
             k: getComputedStyle(document.getElementById('tour-stage')).getPropertyValue('--k') })"
        )
    );
    // Settings opened from the guide (to pick a hotkey, say) and closed again: the same step
    page_js("panel", "tourGo(2), 1");
    on(|c| {
        c.settings_mode = true;
        c.broadcast_state();
    });
    wait(400);
    on(|c| {
        c.settings_mode = false;
        c.broadcast_state();
    });
    wait(400);
    log!("back from Settings: step {} (expect 2)", page_js("panel", "tourAt()"));
    log!("pointer on the clicks, docked right: {} (expect 0,0 each, give or take 1)", tour_pointer());

    // every step at a moment in the middle and at its end, in both themes
    for theme in ["dark", "light"] {
        on(move |c| {
            c.settings.set("theme", json!(theme));
            c.apply_theme();
        });
        wait(700);
        for (i, (id, _, end)) in TOUR_STEPS.iter().enumerate() {
            page_js("panel", &format!("tourGo({i}), 1"));
            wait(500);
            for ms in [2400, *end] {
                tour_freeze(ms);
                wait(200);
                shot(&format!("90-{theme}-{i}{id}-{ms}"));
            }
            log!(
                "guide {theme} step {i} ({id}): misfits {} | scene {}",
                page_js("panel", TOUR_FIT),
                page_js("panel", "document.getElementById('tour-stage').className")
            );
        }
    }

    // every language, in the narrowest panel and the usual one
    for lang in ["th", "en", "zh", "ja", "de"] {
        on(move |c| c.set_pref("lang", json!(lang)));
        wait(700);
        for width in [340.0, 460.0] {
            panel_width(width);
            wait(500);
            let mut problems = Vec::new();
            for (i, (_, _, end)) in TOUR_STEPS.iter().enumerate() {
                page_js("panel", &format!("tourGo({i}), 1"));
                wait(300);
                tour_freeze(*end);
                wait(150);
                let fit = page_js("panel", TOUR_FIT);
                if fit != "\"[]\"" {
                    problems.push(format!("step {i}: {fit}"));
                }
                if width < 400.0 {
                    shot(&format!("91-{lang}-{i}"));
                }
            }
            log!(
                "guide {lang} at {width} px: {} | height {} (expect fits)",
                if problems.is_empty() { "fits".to_string() } else { problems.join(" | ") },
                page_js("panel", "JSON.stringify({ guide: document.querySelector('.tour').scrollHeight, room: document.getElementById('stage').clientHeight, text: document.querySelector('.tour-texts').offsetHeight })")
            );
        }
    }
    on(|c| c.set_pref("lang", json!("th")));
    panel_width(460.0);

    // docked on the left: the scenes mirror, their words don't
    on(|c| {
        c.settings.set("side", json!("left"));
        c.broadcast_state();
    });
    wait(600);
    for (i, (_, _, end)) in TOUR_STEPS.iter().enumerate() {
        page_js("panel", &format!("tourGo({i}), 1"));
        wait(300);
        tour_freeze(*end);
        wait(150);
        shot(&format!("92-left-{i}"));
    }
    log!(
        "docked left: {} (expect the stage mirrored, the text in it not)",
        page_js("panel", "JSON.stringify({ stage: document.getElementById('tour-stage').className, body: document.body.classList.contains('left'), uf: getComputedStyle(document.querySelector('#tour-stage .flip .uf')).transform })")
    );
    log!("pointer on the clicks, docked left: {} (expect 0,0 each, give or take 1)", tour_pointer());
    // the pointer held against the (left) edge while the line grows
    page_js("panel", "tourGo(0), 1");
    wait(250);
    tour_freeze(3000);
    wait(100);
    shot("92-left-hold");
    log!(
        "docked left, holding the edge: pointer {} (expect inside the stage, its tip at the left edge)",
        page_js(
            "panel",
            "(() => { const st = document.getElementById('tour-stage').getBoundingClientRect(), p = document.querySelector('#tour-stage .cur svg').getBoundingClientRect(); \
             return JSON.stringify({ tipFromEdge: Math.round(p.left + 2.25 * p.width / 18 - st.left), inside: p.left >= st.left - 1 && p.right <= st.right + 1 }); })()"
        )
    );
    on(|c| {
        c.settings.set("side", json!("right"));
        c.broadcast_state();
    });

    // "reduce motion" in Windows: one still picture per step
    let emu = page_cdp("panel", "Emulation.setEmulatedMedia", r#"{"features":[{"name":"prefers-reduced-motion","value":"reduce"}]}"#);
    page_js("panel", "tourGo(2), 1");
    wait(500);
    log!(
        "reduce motion ({emu}): {} (expect every animation paused, no pointer)",
        page_js(
            "panel",
            "(() => { const a = document.getElementById('tour-stage').getAnimations({ subtree: true }); \
             return JSON.stringify({ n: a.length, paused: a.every((x) => x.playState === 'paused'), pointer: getComputedStyle(document.querySelector('#tour-stage .cur')).display }); })()"
        )
    );
    shot("93-reduce-motion");
    page_cdp("panel", "Emulation.setEmulatedMedia", r#"{"features":[{"name":"prefers-reduced-motion","value":""}]}"#);

    // the arrow keys and the buttons; what the page would tell ChatDock is caught on the page
    page_js("panel", "tourGo(0), 1");
    page_js(
        "panel",
        "['ArrowRight', 'ArrowRight', 'ArrowLeft', 'ArrowRight'].forEach((key) => document.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))), 1",
    );
    let by_keys = page_js("panel", "tourAt()");
    // past the last step an arrow key does nothing (the scene doesn't start over)
    page_js("panel", "tourGo(4), document.querySelector('#tour-stage > *').dataset.mark = '1', 1");
    page_js("panel", "document.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })), 1");
    let past_end = page_js("panel", "tourAt() + ' ' + !!document.querySelector('#tour-stage > [data-mark]')");
    page_js("panel", "window.__sent = []; window.__send = chatdock.send; chatdock.send = (...a) => window.__sent.push(a); tourGo(0), 1");
    for _ in 0..3 {
        js_click("panel", "#tour-next");
    }
    let before_last = page_js("panel", "document.getElementById('tour-screen').getBoundingClientRect().top");
    js_click("panel", "#tour-next");
    let last = page_js(
        "panel",
        "JSON.stringify({ at: tourAt(), next: document.querySelector('#tour-next .label').textContent, skip: document.getElementById('tour-skip').classList.contains('away'), \
         box: ((r) => !r.hidden && !r.classList.contains('away'))(document.getElementById('autostart-row')), back: !document.getElementById('tour-back').hidden, \
         top: document.getElementById('tour-screen').getBoundingClientRect().top })",
    );
    log!("an arrow key past the last step: {past_end} (expect 4 true) | the stage's top on steps 4 and 5: {before_last} / {last} (expect the same)");
    js_click("panel", "#tour-next");
    let started = page_js("panel", "JSON.stringify(window.__sent.splice(0))");
    page_js("panel", "tourGo(1), 1");
    js_click("panel", "#tour-skip");
    let skipped = page_js("panel", "JSON.stringify(window.__sent.splice(0))");
    page_js("panel", "chatdock.send = window.__send, 1");
    log!(
        "keys → step {by_keys} (expect 2) | last step: {last} (expect 4, เริ่มใช้งานเลย, skip hidden, the box only if Windows start can be set here) | start sends {started} | skip sends {skipped} (expect onboarding:done, the box's tick only with start)"
    );

    // "How to use" later: from the first step, "Close the guide", and the last button goes back to the
    // chat (ChatDock itself this time, with the panel hidden so nothing comes to the front)
    on(|c| {
        c.help_mode = false;
        c.settings.set("onboarded", json!(true));
        c.broadcast_state();
    });
    wait(300);
    tour_show(true);
    wait(800);
    let help = page_js(
        "panel",
        "JSON.stringify({ at: tourAt(), skip: document.getElementById('tour-skip').textContent, top4: (tourGo(3), document.getElementById('tour-screen').getBoundingClientRect().top), \
         last: (tourGo(4), document.querySelector('#tour-next .label').textContent), top5: document.getElementById('tour-screen').getBoundingClientRect().top, \
         box: ((r) => !r.hidden && !r.classList.contains('away'))(document.getElementById('autostart-row')) })",
    );
    on(|c| win32::hide(c.panel.hwnd));
    js_click("panel", "#tour-next");
    wait(600);
    log!(
        "How to use: {help} (expect step 0, ปิดคำแนะนำ, top4 = top5, กลับไปที่แชท, no box) | after its last button: help {} onboarded {} (expect false, true)",
        on(|c| c.help_mode),
        on(|c| c.settings.bool("onboarded"))
    );

    // the header: the highlight glides to the app picked, a new unread number pops (pop-ups and the
    // edge glow off meanwhile, so nothing shows on the screen)
    let apps = on(|c| c.enabled_apps());
    on(|c| {
        c.settings.set("popups", json!(false));
        c.settings.set("glow", json!(false));
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.broadcast_state();
    });
    wait(600);
    let (a0, a1) = (apps[0], apps.get(1).copied().unwrap_or(apps[0]));
    on(move |c| c.set_active(a1, false));
    let moving = page_js("panel", "(() => { const g = document.querySelector('.apps .glider'); return JSON.stringify([g.style.transform, getComputedStyle(g).transitionDuration]); })()");
    wait(700);
    let placed = page_js(
        "panel",
        "(() => { const g = document.querySelector('.apps .glider'), b = document.querySelector('.app.active'); \
         return JSON.stringify({ glider: [g.style.transform, g.style.width, g.hidden], tab: [b.dataset.app, b.offsetLeft, b.offsetWidth], drawn: getComputedStyle(b).backgroundColor }); })()",
    );
    shot("94-header-glide");
    on(move |c| c.set_count(a0, 2));
    wait(80);
    let badge = format!("document.querySelector('.app[data-app=\"{a0}\"] .badge').className");
    let bump = page_js("panel", &badge);
    wait(700);
    let after = page_js("panel", &badge);
    // the tabs come back from behind the volume bar: nothing pops again
    js_click("panel", "#vol");
    wait(200);
    js_click("panel", "#vol-done");
    wait(100);
    let again = page_js("panel", &badge);
    on(move |c| c.set_count(a0, 0));
    log!(
        "header: glider while moving {moving} | in place {placed} (expect the active tab's left and width, the tab itself transparent) | new unread number: {bump} → {after} → after the volume bar {again} (expect badge bump, badge, badge)"
    );
    // the volume bar takes the tabs' place; when it closes the highlight is back on the open app
    js_click("panel", "#vol");
    wait(300);
    let open = page_js(
        "panel",
        "JSON.stringify([!document.getElementById('volbar').hidden, getComputedStyle(document.querySelector('.apps')).display])",
    );
    js_click("panel", "#vol-done");
    wait(300);
    log!(
        "volume bar: open {open} | closed again: {} (expect [true, none], then the glider on the open tab)",
        page_js(
            "panel",
            "(() => { const g = document.querySelector('.apps .glider'), b = document.querySelector('.app.active'); return JSON.stringify({ glider: [g.style.transform, g.style.width, g.hidden], tab: [b.offsetLeft, b.offsetWidth] }); })()"
        )
    );

    // the zoom chip beside the tabs can make the open tab narrower: the highlight follows
    let zoomed = on(|c| c.active());
    let z2 = zoomed.clone();
    on(move |c| {
        c.zoom_step(&zoomed, 1);
        c.zoom_step(&zoomed, 1);
    });
    wait(500);
    let with_chip = page_js(
        "panel",
        "(() => { const g = document.querySelector('.apps .glider'), b = document.querySelector('.app.active'); \
         return JSON.stringify({ chip: !document.getElementById('zoom').hidden, glider: [g.style.transform, g.style.width], tab: [b.offsetLeft, b.offsetWidth] }); })()",
    );
    on(move |c| c.zoom_step(&z2, 0));
    wait(300);
    log!("zoom chip shown: {with_chip} (expect the glider = the open tab)");

    // Facebook's own logo, in the header and on the edge tab
    log!(
        "Facebook's icon: header {} | tab {} (expect #0866ff: the blue f)",
        page_js("panel", "document.querySelector('.app[data-app=facebook] svg path:last-child')?.getAttribute('fill') || 'none'"),
        page_js("tab", "document.querySelector('.app[data-app=facebook] svg path:last-child')?.getAttribute('fill') || 'none'")
    );

    on(move |c| {
        for (k, v) in was {
            c.settings.set(k, v);
        }
        c.apply_theme();
        c.help_mode = false;
        win32::hide(c.panel.hwnd);
        c.broadcast_state();
    });
}

/// Pictures of the guide for a GIF: each step's scene, one picture per 100 ms (stopped at each
/// moment, so the frames come out even), as the panel opens the first time. <shots>/tourgif/
/// s<step>-<ms>.png; the guide's place on the page (CSS px and the scale) goes to the log for
/// cropping.
fn tour_gif_frames() {
    let Some(dir) = on(|c| c.args.shots.clone()) else { return };
    let dir = dir.join("tourgif");
    let _ = std::fs::create_dir_all(&dir);
    // the usual hotkey in the pictures (the test copy can't take it while the real ChatDock holds it:
    // it only has to show)
    on(|c| {
        c.settings.set("hotkey", json!("Control+Alt+C"));
        c.hotkey_ok = true;
    });
    tour_show(false);
    wait(2500);
    log!(
        "guide at {}",
        page_js("panel", "JSON.stringify((() => { const r = document.querySelector('.tour').getBoundingClientRect(); return [r.left, r.top, r.width, r.height, devicePixelRatio]; })())")
    );
    let mut saved = 0;
    for (i, (_, dur, _)) in TOUR_STEPS.iter().enumerate() {
        page_js("panel", &format!("tourGo({i}), 1"));
        wait(500);
        for t in (0..*dur).step_by(100) {
            tour_freeze(t);
            wait(30);
            let Some(w) = rt::app().get_webview_window("panel") else { return };
            let (tx, rx) = mpsc::channel::<String>();
            let path = dir.join(format!("s{i}-{t:05}.png"));
            let _ = w.with_webview(move |pw| unsafe {
                if let Ok(wv) = pw.controller().CoreWebView2() {
                    capture(&wv, path, tx);
                }
            });
            if rx.recv_timeout(Duration::from_secs(5)).is_ok_and(|r| r == "ok") {
                saved += 1;
            }
        }
    }
    log!("guide gif frames: {saved} pictures");
    on(|c| {
        c.help_mode = false;
        win32::hide(c.panel.hwnd);
        c.broadcast_state();
    });
}

// ---------------------------------------------------------------------------------------------
// Game mode and Facebook's number (1.7.4)
// ---------------------------------------------------------------------------------------------
/// Game mode with a pretend game in front: pop-ups wait (nothing on screen), and once the game is
/// gone one card says who wrote (one waiting pop-up shows as itself). The pop-ups are shown off
/// screen; nothing takes the focus.
fn game_mode_test() {
    let was = on(|c| {
        let was = (c.settings.get("popups").clone(), c.settings.get("popupQuietFullscreen").clone(), c.settings.get("lang").clone());
        c.settings.set("popups", json!(true));
        c.settings.set("popupQuietFullscreen", json!(true));
        c.toasts.offscreen = true;
        c.toasts.test_game = Some(true);
        c.toasts_dismiss_all();
        was
    });
    on(|c| c.set_pref("lang", json!("th")));
    log!("game in front for real right now: {} (the user's own screen: expect false unless they are playing)", win32::game_in_front());
    let card = "JSON.stringify([...document.querySelectorAll('.card:not(.leaving)')].map((c) => ({ summary: c.classList.contains('summary'), \
                title: c.querySelector('.title')?.textContent, meta: c.querySelector('.meta')?.textContent, \
                rows: [...c.querySelectorAll('.rows .row')].map((r) => [r.querySelector('.rt b')?.textContent, r.querySelector('.rt span')?.textContent, r.querySelector('.n')?.hidden ? '' : r.querySelector('.n')?.textContent]) })))";

    // three conversations write during the game (one of them twice)
    on(|c| {
        c.counts.insert("instagram".into(), 1);
        c.on_site_notification("discord", 0, "Alice (#general, Gamers)", "gg ez", "", "dc1");
        c.on_site_notification("discord", 0, "Alice (#general, Gamers)", "one more?", "", "dc1");
        c.on_site_notification("x", 0, "Bob", "are you on?", "", "x1");
        let nok =
            crate::inbox::Latest {
                name: "Nok".into(), text: "ไปเล่นกันป่าว".into(), avatar: String::new(), href: "/direct/t/1/".into()
            };
        c.count_popup_for_test("instagram", Some(nok));
    });
    wait(600);
    let during = on(|c| (c.toasts_count(), c.held_count(), win32::is_visible(c.toastwin.hwnd)));
    // the game is still in front a while later: still nothing
    wait(2500);
    let still = on(|c| (c.toasts_count(), c.held_count()));
    on(|c| c.toasts.test_game = Some(false));
    wait(3200);
    let after = on(|c| (c.toasts_count(), c.held_count(), win32::is_visible(c.toastwin.hwnd)));
    shot("95-game-summary");
    log!(
        "game mode: during the game shown/waiting/window {during:?} (expect 0, 3, false) | still in the game {still:?} (expect 0, 3) | after it {after:?} (expect 1, 0, true) | card {}",
        page_js("toasts", card)
    );
    // X read meanwhile (on the phone): its row goes, the card stays with the others
    on(|c| c.toasts_dismiss_app("x"));
    wait(500);
    log!("X read meanwhile: {} (expect the card with 2 rows: Instagram, Discord, and ใหม่ 3)", page_js("toasts", card));
    shot("95-game-summary-2");
    // another game ends while that card is still up: one card with both games' rows
    on(|c| {
        c.toasts.test_game = Some(true);
        c.on_site_notification("x", 0, "Cee", "ready?", "", "x3");
        c.on_site_notification("discord", 0, "Dan", "here", "", "dc3");
    });
    wait(400);
    on(|c| c.toasts.test_game = Some(false));
    wait(3200);
    log!("a second game while the first card is up: {} (expect one card: Dan, Cee, Nok, Alice; ใหม่ 5)", page_js("toasts", card));
    on(|c| c.toasts_dismiss_all());
    wait(700);

    // Facebook's title blinks during the game (its number gone 3 s, then back): what waits stays
    on(|c| {
        c.toasts.test_game = Some(true);
        c.counted_pages.insert("facebook".into());
        c.site_counts.insert("facebook".into(), 10);
        c.settings.set_in("seenCounts", "facebook", json!(9));
        c.counts.insert("facebook".into(), 1);
        let beam = crate::inbox::Latest { name: "Beam".into(), text: "brb".into(), avatar: String::new(), href: "/messages/t/9/".into() };
        c.count_popup_for_test("facebook", Some(beam));
        c.on_title("facebook", "Facebook");
    });
    wait(3500);
    let gone = on(|c| (c.counts.get("facebook").copied().unwrap_or(0), c.held_count()));
    on(|c| c.on_title("facebook", "(10) Facebook"));
    wait(300);
    let back = on(|c| (c.counts.get("facebook").copied().unwrap_or(0), c.held_count()));
    on(|c| c.toasts.test_game = Some(false));
    wait(3200);
    log!("Facebook blink in a game: number gone → (unread, waiting) {gone:?} (expect 0, 1) | back → {back:?} (expect 1, 1) | after the game: {} (expect Beam's own card)", page_js("toasts", card));
    on(|c| {
        c.toasts_dismiss_all();
        c.set_count("facebook", 0);
    });
    wait(700);

    // an update found during the game: no pop-up over it (it comes later)
    on(|c| {
        c.toasts.test_game = Some(true);
        c.update_announced.clear();
        c.announce_update("9.9.9", true);
    });
    wait(400);
    log!("update during a game: shown {} announced {:?} (expect 0, \"\")", on(|c| c.toasts_count()), on(|c| c.update_announced.clone()));
    on(|c| c.toasts.test_game = Some(false));
    wait(700);

    // just one conversation waited: it shows as its own pop-up
    on(|c| {
        c.toasts.test_game = Some(true);
        c.on_site_notification("discord", 0, "Mint", "gg", "", "dc2");
        c.on_site_notification("discord", 0, "Mint", "again?", "", "dc2");
    });
    wait(400);
    on(|c| c.toasts.test_game = Some(false));
    wait(3200);
    log!("one conversation waited: {} (expect a normal card: Mint, again?, 2 unread)", page_js("toasts", card));
    on(|c| c.toasts_dismiss_all());
    wait(700);

    // read during the game (the app opened, or on the phone): nothing left to show afterwards
    on(|c| {
        c.toasts.test_game = Some(true);
        c.counts.insert("instagram".into(), 1);
        let nok = crate::inbox::Latest { name: "Nok".into(), text: "hi".into(), avatar: String::new(), href: String::new() };
        c.count_popup_for_test("instagram", Some(nok));
    });
    wait(300);
    let held = on(|c| c.held_count());
    on(|c| {
        c.set_count("instagram", 0);
        c.toasts.test_game = Some(false);
    });
    wait(3200);
    log!("read during the game: waiting {held} → after it shown {} (expect 1, 0)", on(|c| c.toasts_count()));

    // game mode off: pop-ups show right away, game or not
    on(|c| {
        c.settings.set("popupQuietFullscreen", json!(false));
        c.toasts.test_game = Some(true);
        c.on_site_notification("x", 0, "Bob", "now?", "", "x2");
    });
    wait(500);
    log!("game mode off, in a game: shown {} waiting {} (expect 1, 0)", on(|c| c.toasts_count()), on(|c| c.held_count()));
    on(move |c| {
        c.toasts_dismiss_all();
        c.toasts.test_game = None;
        c.toasts.offscreen = false;
        c.settings.set("popups", was.0);
        c.settings.set("popupQuietFullscreen", was.1);
        c.settings.set("lang", was.2);
    });
    wait(700);
}

/// Game mode switched on: its explainer opens (off screen here), with its three scenes and "don't
/// show this again"; ticked, switching it on again opens nothing, but Settings' "How it works" still
/// does. Pictures of every scene, one per 100 ms, for a GIF: <shots>/gmintro/s<scene>-<ms>.png.
fn game_intro_test() {
    let was = on(|c| {
        let was = (c.settings.get("popupQuietFullscreen").clone(), c.settings.get("gameModeIntro").clone());
        c.whatsnew.offscreen = true;
        c.settings.set("popupQuietFullscreen", json!(false));
        c.settings.set("gameModeIntro", json!(true));
        was
    });
    on(|c| c.set_pref("popupQuietFullscreen", json!(true)));
    wait(2500);
    let made = on(|c| c.whatsnew.win.is_some());
    log!(
        "game mode switched on: explainer {made} | {} (expect true | intro, the title, 4 lines, 3 scenes, the box shown, no GitHub link)",
        page_js(
            "whatsnew",
            "JSON.stringify({ intro: document.body.classList.contains('intro'), title: document.getElementById('title').textContent, sub: document.getElementById('route').textContent, \
             lines: document.querySelectorAll('.notes li').length, scenes: document.querySelectorAll('#dots .dot').length, box: !document.getElementById('again').hidden, \
             boxText: document.querySelector('#again span').textContent, github: !document.getElementById('github').hidden, ok: document.getElementById('ok').textContent, \
             card: document.querySelector('.card').offsetHeight, fits: document.getElementById('notes').scrollHeight <= document.getElementById('notes').clientHeight + 1 })"
        )
    );
    // every scene, frame by frame (stopped at each moment)
    let dir = on(|c| c.args.shots.clone()).map(|d| d.join("gmintro"));
    if let Some(dir) = &dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut saved = 0;
    for i in 0..3 {
        page_js("whatsnew", &format!("showDemo({i}, true), clearTimeout(demoTimer), 1"));
        wait(400);
        for t in (0..6000).step_by(100) {
            page_js(
                "whatsnew",
                &format!(
                    "(() => {{ clearTimeout(demoTimer); const a = document.getAnimations().filter((x) => x.effect && x.effect.target && scene.contains(x.effect.target)); \
                     a.forEach((x) => {{ x.pause(); x.currentTime = {t}; }}); return a.length; }})()"
                ),
            );
            wait(30);
            if t % 1500 == 0 {
                shot(&format!("96-gmintro-{i}-{t:04}"));
            }
            let Some(dir) = dir.clone() else { continue };
            let Some(w) = rt::app().get_webview_window("whatsnew") else { break };
            let (tx, rx) = mpsc::channel::<String>();
            let path = dir.join(format!("s{i}-{t:05}.png"));
            let _ = w.with_webview(move |pw| unsafe {
                if let Ok(wv) = pw.controller().CoreWebView2() {
                    capture(&wv, path, tx);
                }
            });
            if rx.recv_timeout(Duration::from_secs(5)).is_ok_and(|r| r == "ok") {
                saved += 1;
            }
        }
    }
    log!(
        "explainer frames: {saved} | card at {}",
        page_js("whatsnew", "JSON.stringify((() => { const r = document.querySelector('.card').getBoundingClientRect(); return [r.left, r.top, r.width, r.height, devicePixelRatio]; })())")
    );
    // "don't show this again", then Got it
    page_js("whatsnew", "document.getElementById('again-box').checked = true, document.getElementById('ok').click(), 1");
    wait(800);
    let after = on(|c| (c.whatsnew.win.is_some(), c.settings.bool("gameModeIntro")));
    // off and on again: nothing opens now
    on(|c| {
        c.set_pref("popupQuietFullscreen", json!(false));
        c.set_pref("popupQuietFullscreen", json!(true));
    });
    wait(1500);
    let again = on(|c| c.whatsnew.win.is_some());
    // Settings → Notifications → "How it works": it opens all the same
    on(|c| c.settings_action_test("game-intro", serde_json::Value::Null));
    wait(2000);
    let how = on(|c| c.whatsnew.win.is_some());
    js_click("whatsnew", "#ok");
    wait(800);
    log!(
        "explainer: after Got it with the box ticked: open {}, show again {} (expect false, false) | switched on again: opens {again} (expect false) | How it works: opens {how}, then closed {} and show again {} (expect true, true, true)",
        after.0,
        after.1,
        on(|c| c.whatsnew.win.is_none()),
        on(|c| c.settings.bool("gameModeIntro"))
    );
    on(move |c| {
        c.close_whats_new(false);
        c.whatsnew.offscreen = false;
        c.settings.set("popupQuietFullscreen", was.0);
        c.settings.set("gameModeIntro", was.1);
    });
}

/// Facebook's number: a blink of its title (gone, then back as high) changes nothing, and a rise
/// with nothing unread in its chat list and no "… sent you a message" isn't a chat. The list is a
/// stand-in put into the (logged-out) test page; pop-ups are off, so nothing shows.
fn facebook_count_test() {
    let popups_were = on(|c| {
        let was = c.settings.get("popups").clone();
        c.settings.set("popups", json!(false));
        was
    });
    let fb = "facebook";
    // what Facebook counted, and what was seen of it: 10, 9 seen, 1 unread (10-05 11:51)
    let start = move |c: &mut Core| {
        c.counted_pages.insert(fb.into());
        c.site_counts.insert(fb.into(), 10);
        c.settings.set_in("seenCounts", fb, json!(9));
        c.counts.insert(fb.into(), 1);
        c.count_drops.remove(fb);
        c.last_flash.remove(fb);
        c.load_started_at.insert(fb.into(), 0);
        if let Some(t) = c.fallback_timers.remove(fb) {
            rt::cancel(t);
        }
    };
    let state = move || {
        on(move |c| {
            format!(
                "unread {} seen {} pop-up coming {}",
                c.counts.get(fb).copied().unwrap_or(0),
                c.settings.get("seenCounts").get(fb).and_then(serde_json::Value::as_u64).unwrap_or(0),
                c.fallback_timers.contains_key(fb)
            )
        })
    };

    // 1. the title loses its number for 3 s and more, then "(10)" again
    on(start);
    on(move |c| c.on_title(fb, "Facebook"));
    wait(3400);
    let gone = state();
    on(move |c| c.on_title(fb, "(10) Facebook"));
    wait(200);
    log!(
        "Facebook blink: number gone → {gone} (expect unread 0 seen 0) | back as (10) → {} (expect unread 1 seen 9, no pop-up coming)",
        state()
    );

    // 2. gone, then read on the phone and one new message: "(1)" is news
    on(start);
    on(move |c| c.on_title(fb, "Facebook"));
    wait(3400);
    on(move |c| c.on_title(fb, "(1) Facebook"));
    wait(200);
    log!("Facebook read elsewhere, then one new: {} (expect unread 1 seen 0)", state());

    // 2b. Instagram: read on the phone, the reply comes back as "(1)": news, not a blink
    let ig = "instagram";
    on(move |c| {
        c.counted_pages.insert(ig.into());
        c.site_counts.insert(ig.into(), 1);
        c.settings.set_in("seenCounts", ig, json!(0));
        c.counts.insert(ig.into(), 1);
        c.load_started_at.insert(ig.into(), 0);
        if let Some(t) = c.fallback_timers.remove(ig) {
            rt::cancel(t);
        }
        c.on_title(ig, "Instagram");
    });
    wait(3400);
    on(move |c| c.on_title(ig, "(1) Instagram"));
    wait(200);
    log!(
        "Instagram read elsewhere, then a reply: unread {} pop-up coming {} (expect 1, true)",
        on(move |c| c.counts.get(ig).copied().unwrap_or(0)),
        on(move |c| c.fallback_timers.contains_key(ig))
    );
    on(move |c| c.set_count(ig, 0));

    // 3. a like: "(11)", nothing unread in the chat list, no message in the title
    let list = |weights: [u32; 4]| {
        format!(
            "(() => {{ document.getElementById('cd-list')?.remove(); const box = document.createElement('div'); box.id = 'cd-list'; box.style.cssText = 'position:fixed;left:0;top:0;width:320px;z-index:2147483647;background:#fff;font:14px sans-serif'; \
             [['Somchai Jaidee', 'You: ok see you'], ['Nok', 'see you tonight'], ['Beam', 'haha'], ['Old friend', 'long ago']].forEach(([name, text], i) => {{ \
               const r = document.createElement('a'); r.href = '/messages/t/' + (i + 1) + '/'; r.style.cssText = 'display:flex;gap:8px;height:64px;align-items:center'; \
               const img = document.createElement('img'); img.src = 'https://example.com/p' + i + '.jpg'; img.width = 44; img.height = 44; \
               const col = document.createElement('div'); const n = document.createElement('span'); n.textContent = name; const t = document.createElement('span'); t.textContent = text; t.style.fontWeight = {weights}[i]; t.style.display = 'block'; \
               col.append(n, t); r.append(img, col); r.addEventListener('click', (e) => e.preventDefault()); box.append(r); }}); \
             document.body.append(box); return 1; }})()",
            weights = json!(weights)
        )
    };
    // (on a page of chats, as Facebook's always is: a conversation with the list beside it)
    view_js(fb, "history.pushState({}, '', '/messages/t/1/'), 1");
    view_js(fb, &list([400, 400, 400, 400]));
    on(start);
    on(move |c| c.on_title(fb, "(11) Facebook"));
    wait(200);
    let rose = state();
    wait(3600);
    log!(
        "Facebook like: number up → {rose} (expect unread 2, a check coming) | its list has nothing unread → {} (expect unread 0 seen 11)",
        state()
    );

    // 4. the same, but the title said "… sent you a message": it is a chat
    on(start);
    on(move |c| {
        c.on_title(fb, "Somchai sent you a message");
        c.on_title(fb, "(11) Facebook");
    });
    wait(3800);
    log!("Facebook message the list doesn't show yet: {} (expect unread 2 seen 9: still counted)", state());

    // 5. one unread chat in the list: the number is that one (the rest are notifications)
    view_js(fb, &list([400, 700, 400, 400]));
    on(start);
    on(move |c| c.on_title(fb, "(11) Facebook"));
    wait(3800);
    log!("Facebook with an unread chat: {} (expect unread 1 seen 10)", state());

    // 6. 10-05 15:39, at the start: Facebook counts 3, nothing seen yet, one chat unread
    on(move |c| {
        start(c);
        c.site_counts.insert(fb.into(), 0);
        c.settings.set_in("seenCounts", fb, json!(0));
        c.counts.insert(fb.into(), 0);
        c.on_title(fb, "(3) Facebook");
    });
    wait(3800);
    log!("Facebook at the start, 3 counted, one chat unread: {} (expect unread 1 seen 2)", state());
    view_js(fb, "document.getElementById('cd-list')?.remove(), 1");
    on(move |c| {
        c.settings.set("popups", popups_were);
        c.set_count(fb, 0);
    });
}

/// The pre-release review's findings, checked (nothing takes the focus or the keyboard; pop-ups off).
fn review_fixes_test() {
    // 1. a Discord title longer than 90 characters still says which server
    let long = "Alice (#announcements-and-patch-notes, Official Valorant Thailand Community Server | Thai Players)";
    let popups_was = on(|c| c.settings.get("popups").clone());
    on(move |c| {
        c.settings.set("popups", json!(false));
        c.settings.set("discordServers", json!({}));
        c.on_site_notification("discord", 0, long, "gg", "", "r1");
    });
    let listed = on(|c| c.settings.get("discordServers").to_string());
    on(move |c| c.settings.set("popups", popups_was));
    // 7. the mic and sound are the last two switches (a voice channel's own come first)
    view_js(
        "discord",
        "(() => { document.querySelectorAll('section.panels_test').forEach((x) => x.remove()); const s = document.createElement('section'); s.className = 'panels_test'; \
         for (let i = 0; i < 4; i++) { const b = document.createElement('button'); b.setAttribute('role', 'switch'); b.setAttribute('aria-checked', 'false'); \
         b.onclick = () => b.setAttribute('aria-checked', b.getAttribute('aria-checked') === 'true' ? 'false' : 'true'); s.append(b); } \
         document.body.append(s); return 1; })()",
    );
    on(|c| c.discord_voice(0));
    wait(900);
    let after_mute = view_js(
        "discord",
        "JSON.stringify([...document.querySelectorAll('section.panels_test button')].map((b) => b.getAttribute('aria-checked')))",
    );
    on(|c| c.discord_voice(1));
    wait(900);
    let after_deafen = view_js(
        "discord",
        "JSON.stringify([...document.querySelectorAll('section.panels_test button')].map((b) => b.getAttribute('aria-checked')))",
    );
    on(|c| c.toasts_dismiss_all());
    // 2. the call icon of an app with a call window gives that window; other icons, the panel
    let picks = on(|c| {
        c.call_windows.push((c.tab.hwnd, "instagram".into())); // any real window will do
        let r = (
            c.tab_call_window("instagram", true).is_some(),
            c.tab_call_window("instagram", false).is_some(),
            c.tab_call_window("discord", true).is_some(),
        );
        c.call_windows.clear();
        r
    });
    // 4. the voice key dropdowns: the other one's key can't be picked; a refused one comes back
    on(|c| {
        c.settings.set("discordMuteKey", json!("Control+Alt+M"));
        c.settings.set("discordDeafenKey", json!(""));
        c.settings_mode = true;
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.broadcast_state();
        c.emit("panel", "settings:goto", json!(["discord"]));
    });
    wait(1200);
    let disabled = page_js(
        "panel",
        "JSON.stringify([...document.querySelectorAll('[data-voice-keys]')].map((s) => s.dataset.pref + ':' + [...s.options].filter((o) => o.disabled).map((o) => o.value).join('/')))",
    );
    let refused = on(|c| c.set_pref("discordDeafenKey", json!("Control+Alt+M")));
    on(|c| {
        c.settings.set("discordMuteKey", json!(""));
        c.settings_mode = false;
        win32::hide(c.panel.hwnd);
        c.broadcast_state();
    });
    log!(
        "review fixes: long title's server listed {listed} | after the mute key {after_mute}, after the deafen key {after_deafen} | call window for (the call icon, an app icon, no call window) {picks:?} | taken keys {disabled}, the same key twice refused {} (expect the long server listed, [false,false,true,false], [false,false,true,true], (true, false, false), deafen:Control+Alt+M, true)",
        !refused
    );
    // A sign-out or shutdown that was cancelled (an app wouldn't close): ChatDock carries on, so a
    // WebView2 crash after it restarts ChatDock instead of quitting
    let ending = on(|c| unsafe {
        let h = win32::h(c.panel.hwnd);
        SendMessageW(h, WM_QUERYENDSESSION, None, None);
        let asked = SESSION_ENDING.load(Ordering::SeqCst);
        SendMessageW(h, WM_ENDSESSION, Some(WPARAM(0)), None);
        (asked, SESSION_ENDING.load(Ordering::SeqCst))
    });
    log!("a sign-out asked for, then cancelled: ending {} -> {} (expect true -> false)", ending.0, ending.1);
}

/// Each app's volume: what the page plays is the site's volume times ChatDock's, the site still
/// reads back its own, and Web Audio goes through ChatDock's gain. Nothing makes a sound (nothing
/// plays). In Discord's page.
fn volume_test() {
    let state = || {
        on(|c| {
            c.volume_states.remove("discord");
            c.chats.post_json("discord", &json!({ "type": format!("{}Check", crate::chats::volume_key()) }));
        });
        wait(400);
        on(|c| c.volume_states.get("discord").cloned().unwrap_or_default())
    };
    let made = view_js(
        "discord",
        "(() => { const a = document.createElement('audio'); a.volume = 0.8; window.__va = a; const ctx = new AudioContext(); \
         const o = ctx.createOscillator(); const r = o.connect(ctx.destination); window.__vctx = [ctx, o]; return r === ctx.destination && a.volume === 0.8; })()",
    );
    let full = state();
    on(|c| c.set_app_volume("discord", 50.0));
    wait(300);
    let half = state();
    let site_reads = view_js("discord", "window.__va.volume");
    let disconnect = view_js(
        "discord",
        "(() => { try { const [ctx, o] = window.__vctx; o.disconnect(ctx.destination); o.connect(ctx.destination); o.disconnect(); return 'ok'; } catch (e) { return 'threw ' + e.message; } })()",
    );
    // a new page starts at the app's volume too
    on(|c| c.set_app_volume("discord", 30.0));
    view_js("discord", "location.reload(), 1");
    wait(4000);
    view_js("discord", "(() => { const b = new Audio(); b.volume = 1; window.__vb = b; return 1; })()");
    let reloaded = state();
    let saved = on(|c| (c.app_volume("discord"), c.ui_state()["volumes"]["discord"].clone()));
    on(|c| c.set_app_volume("discord", 100.0));
    log!(
        "volume: made {made} | at 100% {full} | at 50% {half} (the site reads {site_reads}) | disconnect {disconnect} | after a reload at 30% {reloaded} | saved {saved:?} (expect true, real .8 gain 1, real .4 seenBySite .8 gain .5, .8, ok, real .3, (30, 30))"
    );
}

/// Spotify: its page, protected media in it (Widevine), its sign-in pages, and song titles that
/// never count as unread messages.
fn spotify_test() {
    use crate::apps::keep_inside;
    let inside = [
        keep_inside("spotify", "https://accounts.spotify.com/en/login"),
        keep_inside("spotify", "https://accounts.google.com/o/oauth2/v2/auth?client_id=x"),
        keep_inside("spotify", "https://www.google.com/search?q=x"),
        keep_inside("x", "https://accounts.google.com/"),
        keep_inside("discord", "https://accounts.google.com/"),
    ];
    let was = on(|c| {
        let was = c.is_enabled("spotify");
        c.set_app_enabled("spotify", true);
        c.on_title("spotify", "(3) Song • Artist");
        (was, c.site_counts.get("spotify").copied().unwrap_or(0))
    });
    wait(12000); // it loads
    let info = on(|c| (c.load_state.get("spotify").copied().unwrap_or("?"), c.chats.source("spotify"), c.chats.has("spotify")));
    let title = view_js("spotify", "document.title");
    // a load replaced by the next one right away: no error screen, no reload
    view_js("spotify", "(() => { location.href = 'https://open.spotify.com/search'; setTimeout(() => { location.href = 'https://open.spotify.com/'; }, 40); return 1; })()");
    wait(9000);
    let replaced = on(|c| c.load_state.get("spotify").copied().unwrap_or("?"));
    let links = [
        crate::chats::opens_outside("https://example.com/", false),
        crate::chats::opens_outside("spotify:track:1", false),
        crate::chats::opens_outside("spotify:track:1", true),
        crate::chats::opens_outside("mailto:a@b.c", true),
    ];
    let drm = view_js_async(
        "spotify",
        "navigator.requestMediaKeySystemAccess('com.widevine.alpha', [{ initDataTypes: ['cenc'], audioCapabilities: [{ contentType: 'audio/mp4; codecs=\"mp4a.40.2\"' }] }]).then(() => 'widevine', (e) => 'no: ' + e.message)",
    );
    // a picture of it (the panel off screen, never activated)
    let active_was = on(|c| {
        let was = c.active();
        c.set_active("spotify", false);
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.panel_state = PanelState::Open;
        c.layout_views();
        c.broadcast_state();
        was
    });
    wait(2500);
    shot("85-spotify");
    on(move |c| {
        c.panel_state = PanelState::Hidden;
        win32::hide(c.panel.hwnd);
        c.set_active(&active_was, false);
        c.layout_views();
        c.broadcast_state();
    });
    if !was.0 {
        on(|c| c.set_app_enabled("spotify", false));
    }
    log!(
        "spotify: stays inside (its login, Google sign-in, Google search, X's Google sign-in, Discord to Google) {inside:?} | a song title's number {} | page {info:?} {title} | {drm} | two loads at once: {replaced} | outside (a web page nobody clicked, a spotify: link by itself, clicked, a clicked mailto:) {links:?} (expect [true, true, false, true, false], 0, loaded open.spotify.com, widevine, ready, [false, false, true, true])",
        was.1
    );
}

/// The real way a site's notification reaches ChatDock: the page itself raises one (as Discord
/// does), WebView2 hands it over, and a Discord server's shows up in Settings. Pop-ups off.
fn notification_path_test() {
    let perms: Vec<String> =
        on(|c| c.enabled_apps()).into_iter().map(|id| format!("{id} {}", view_js(id, "Notification.permission"))).collect();
    let popups_was = on(|c| {
        let was = c.settings.get("popups").clone();
        c.settings.set("popups", json!(false));
        c.settings.set("discordServers", json!({}));
        was
    });
    let made = view_js(
        "discord",
        "(() => { try { const n = new Notification('Alice (#general, Test Server)', { body: 'hello', tag: 'cd-test' }); window.__cdN = n; return 'made'; } catch (e) { return 'threw ' + e.message; } })()",
    );
    wait(1500);
    let listed = on(|c| c.settings.get("discordServers").to_string());
    // a service worker's notification (WhatsApp's way) goes the same way
    let sw = view_js_async(
        "discord",
        "navigator.serviceWorker && navigator.serviceWorker.getRegistration ? navigator.serviceWorker.getRegistration().then((r) => r ? 'has a service worker' : 'no service worker') : 'no api'",
    );
    on(move |c| c.settings.set("popups", popups_was));
    log!("notification path: permission {perms:?} | page notification {made} | Discord servers now {listed} | {sw} (expect granted everywhere, made, {{\"Test Server\":true}})");
}

/// 1.7.1's screens: Discord's servers read from its sidebar (a stand-in for it here), the "Discord
/// sleeps" warning and its button, and the header's volume (the panel off screen, never
/// activated).
fn ui171_test() {
    // a stand-in for Discord's server sidebar: two servers, the home button and the "add" button
    on(|c| c.settings.set("discordServers", json!({})));
    view_js(
        "discord",
        "(() => { const nav = document.createElement('nav'); nav.id = 'cd-guilds'; \
         nav.innerHTML = '<div data-dnd-name=\"Gamers\"><div data-list-item-id=\"guildsnav___1111\"></div></div>' \
         + '<div data-list-item-id=\"guildsnav___2222\"><span data-dnd-name=\"My Server\"></span></div>' \
         + '<div data-list-item-id=\"guildsnav___home\" data-dnd-name=\"Direct Messages\"></div>' \
         + '<div data-list-item-id=\"guildsnav___create-join-button\"></div>'; document.body.append(nav); return 1; })()",
    );
    on(|c| c.read_discord_servers());
    wait(800);
    let listed = on(|c| c.settings.get("discordServers").to_string());

    // the panel out off screen, on Discord's settings page, Discord set to sleep
    let (active_was, sleep_was) = on(|c| {
        let was = (c.active(), c.settings.app_pref("discord", "sleep"));
        c.settings.set_app_pref("discord", "sleep", true);
        c.set_active("discord", false);
        c.settings_mode = true;
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.panel_state = PanelState::Open;
        c.broadcast_state();
        c.emit("panel", "settings:goto", json!(["discord"]));
        was
    });
    wait(1500);
    shot("86-discord-page");
    let warning = page_js("panel", "!document.querySelector('.dc-sleep').hidden");
    let pressed = js_click("panel", "[data-action=\"discord-awake\"]");
    wait(500);
    let awake = on(|c| !c.settings.app_pref("discord", "sleep"));

    // the header's volume: closed, then open at 60 %, then the wheel on the speaker
    on(|c| {
        c.settings_mode = false;
        c.set_app_volume("discord", 60.0);
        c.layout_views();
        c.broadcast_state();
    });
    wait(800);
    shot("87-volume-closed");
    let icon = page_js("panel", "document.getElementById('vol').dataset.icon + ' | ' + document.getElementById('vol').title");
    js_click("panel", "#vol");
    wait(600);
    shot("88-volume-open");
    let open = page_js("panel", "JSON.stringify({ bar: !document.getElementById('volbar').hidden, tabs: getComputedStyle(document.querySelector('.apps')).display, value: document.getElementById('vol-range').value, num: document.getElementById('vol-num').textContent })");
    page_js(
        "panel",
        "document.getElementById('vol').dispatchEvent(new WheelEvent('wheel', { deltaY: -100, bubbles: true, cancelable: true })), 1",
    );
    wait(400);
    let after_wheel = on(|c| c.app_volume("discord"));
    js_click("panel", "#vol-mute");
    wait(400);
    let muted = on(|c| (c.settings.app_pref("discord", "sound"), c.ui_state()["soundOn"]["discord"].clone()));
    shot("89-volume-muted");
    js_click("panel", "#vol-mute");
    js_click("panel", "#vol-done");
    wait(300);
    let closed = page_js("panel", "document.getElementById('volbar').hidden");
    on(move |c| {
        c.set_app_volume("discord", 100.0);
        c.settings.set_app_pref("discord", "sleep", sleep_was);
        c.panel_state = PanelState::Hidden;
        win32::hide(c.panel.hwnd);
        c.set_active(&active_was, false);
        c.layout_views();
        c.broadcast_state();
    });
    log!(
        "1.7.1 screens: servers from the sidebar {listed} | sleep warning {warning}, button pressed {pressed}, awake {awake} | speaker {icon} | open {open} | wheel up -> {after_wheel} | mute (sound on, state) {muted:?} | closed {closed} (expect Gamers + My Server only, true, true, true, volume at 60 %, bar shown + tabs hidden + 60, 65, (false, false), true)"
    );
}

/// 1.7.1's pre-release review, checked: the volume in a frame and on media that only got a source,
/// the chat-wide mute and Spotify, the tab leaving a sign-in page, Facebook sign-in pages, the
/// header with many apps, and the volume bar closing with the chat. Nothing takes the focus.
fn review171_test() {
    // frames get the app's volume: X's view on example.com (a page that may be framed), with a
    // frame of the same site so the test can look inside
    on(|c| c.chats.navigate("x", "https://example.com/"));
    wait(4000);
    view_js("x", "(() => { const f = document.createElement('iframe'); f.id = 'cd-frame'; f.src = '/?frame'; document.body.append(f); return 1; })()");
    wait(2500);
    on(|c| c.set_app_volume("x", 40.0));
    wait(600);
    let frame = view_js("x", "(() => { try { return String(document.getElementById('cd-frame').contentWindow[Symbol.for('chatdock.volume')]); } catch (e) { return 'threw ' + e.message; } })()");
    // a frame made after the level was set asks for it
    view_js("x", "(() => { const f = document.createElement('iframe'); f.id = 'cd-frame2'; f.src = '/?later'; document.body.append(f); return 1; })()");
    wait(2500);
    let late_frame = view_js("x", "(() => { try { return String(document.getElementById('cd-frame2').contentWindow[Symbol.for('chatdock.volume')]); } catch (e) { return 'threw ' + e.message; } })()");
    on(|c| c.set_app_volume("x", 100.0));
    on(|c| c.set_app_volume("discord", 40.0));
    // media that only got a source (no play(), no volume): at the app's volume
    view_js("discord", "(() => { const a = new Audio(); a.src = 'data:,'; window.__vs = a; return 1; })()");
    let state = {
        on(|c| {
            c.volume_states.remove("discord");
            c.chats.post_json("discord", &json!({ "type": format!("{}Check", crate::chats::volume_key()) }));
        });
        wait(400);
        on(|c| c.volume_states.get("discord").cloned().unwrap_or_default())
    };
    on(|c| c.set_app_volume("discord", 100.0));

    // "all chat sounds off" leaves Spotify alone
    let muted = on(|c| {
        let spotify = c.is_enabled("spotify");
        if !spotify {
            c.set_app_enabled("spotify", true);
        }
        c.set_pref("muted", json!(true));
        spotify
    });
    wait(6000);
    let (discord_muted, spotify_muted) = on(|c| (c.chats.is_muted("discord"), c.chats.is_muted("spotify")));
    on(|c| c.set_pref("muted", json!(false)));

    // the Spotify tab from a sign-in page goes back to Spotify
    view_js("spotify", "location.href = 'https://accounts.google.com/', 1");
    wait(5000);
    let away = on(|c| c.chats.source("spotify"));
    on(|c| c.go_home("spotify"));
    wait(5000);
    let back = on(|c| c.chats.source("spotify"));

    // Facebook: its sign-in pages are a sign-in; an ordinary Facebook link isn't
    let facebook = [
        crate::apps::is_auth_popup("spotify", "https://www.facebook.com/v19.0/dialog/oauth?client_id=1"),
        crate::apps::is_auth_popup("spotify", "https://www.facebook.com/login.php"),
        crate::apps::is_auth_popup("spotify", "https://www.facebook.com/SomeArtist"),
        crate::apps::keep_inside("spotify", "https://www.facebook.com/SomeArtist"),
    ];

    // the header with seven apps in a narrow panel: the tabs stop before the buttons; the volume
    // bar closes with the chat
    let apps_were = on(|c| c.settings.get("apps").clone());
    on(|c| {
        for id in ["telegram", "whatsapp", "spotify"] {
            c.set_app_enabled(id, true);
        }
        c.set_active("discord", false);
        let d = c.target_display();
        let mut g = c.panel_geometry(&d);
        g.w = (340.0 * d.ui).round() as i32;
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.panel_state = PanelState::Open;
        c.broadcast_state();
    });
    wait(1200);
    let header = page_js(
        "panel",
        "(() => { const a = document.querySelector('.apps').getBoundingClientRect(), v = document.getElementById('vol').getBoundingClientRect(); return JSON.stringify({ tabsEnd: Math.round(a.right), speakerStart: Math.round(v.left), clear: a.right <= v.left }); })()",
    );
    shot("90-header-crowded");
    js_click("panel", "#vol");
    wait(300);
    on(|c| {
        c.panel_state = PanelState::Hidden;
        win32::hide(c.panel.hwnd);
        c.broadcast_state();
    });
    wait(500);
    let bar_closed = page_js("panel", "document.getElementById('volbar').hidden");
    on(move |c| {
        c.settings.set("apps", apps_were);
        c.set_active("instagram", false);
        c.broadcast_state();
    });
    log!(
        "1.7.1 review: frame {frame}, a later frame {late_frame} | source only {state} | all sounds off: discord muted {discord_muted}, spotify muted {spotify_muted} (spotify was on {muted}) | tab from a sign-in page: {away} -> {back} | facebook (dialog, login, a page, stays inside) {facebook:?} | header {header} | bar closed with the chat {bar_closed} (expect 0.4, 0.4, real 0.4, true, false, accounts.google.com -> open.spotify.com, [true, true, false, false], clear, true)"
    );
}

/// Who wrote, read from the chat list (inbox.rs), on stand-in lists put into the test profile's
/// Facebook view (rows that link to their conversation, the newest unread one bold) and Instagram
/// view (rows that are buttons with a picture): what the pop-up says, the conversation a click on it
/// opens, the newest unread one the edge tab opens, and the way back to the list (not while typing).
/// Nothing takes the focus; the pop-ups show for a moment.
fn inbox_test() {
    // A list: rows [name, last message, bold], each a link or a button; a click is only noted.
    let list = |links: bool| {
        format!(
            "(() => {{ document.getElementById('cd-list')?.remove(); const box = document.createElement('div'); box.id = 'cd-list'; box.style.cssText = 'position:fixed;left:0;top:0;width:320px;z-index:2147483647;background:#fff;font:14px sans-serif'; \
             const rows = [['Somchai Jaidee', 'You: ok see you', 400], ['Nok', 'ไปเล่นกันป่าว คืนนี้', 700], ['Beam', 'haha', 700], ['Old friend', 'long ago', 400]]; \
             rows.forEach(([name, text, w], i) => {{ const r = document.createElement({tag}); {attrs} r.style.cssText = 'display:flex;gap:8px;height:64px;align-items:center'; \
               const img = document.createElement('img'); img.src = 'https://example.com/p' + i + '.jpg'; img.width = 44; img.height = 44; \
               const col = document.createElement('div'); const n = document.createElement('span'); n.textContent = name; const t = document.createElement('span'); t.textContent = text; t.style.fontWeight = w; t.style.display = 'block'; \
               col.append(n, t); r.append(img, col); r.addEventListener('click', (e) => {{ e.preventDefault(); window.__cdOpened = name; }}); box.append(r); }}); \
             const home = document.createElement('a'); home.href = '/messages/'; home.textContent = 'Chats'; home.style.cssText = 'display:block;height:20px'; home.addEventListener('click', (e) => {{ e.preventDefault(); window.__cdBack = true; }}); box.prepend(home); \
             document.body.append(box); window.__cdOpened = ''; window.__cdBack = false; return 1; }})()",
            tag = if links { "'a'" } else { "'div'" },
            attrs = if links { "r.href = '/messages/t/' + (i + 1) + '/';" } else { "r.setAttribute('role', 'button');" },
        )
    };
    let read = |app: &'static str| {
        let (tx, rx) = mpsc::channel();
        on(move |c| {
            c.read_latest(
                app,
                Box::new(move |_, latest| {
                    let _ = tx.send(latest);
                }),
            )
        });
        rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
    };
    let popups_were = on(|c| {
        let was = c.settings.get("popups").clone();
        c.settings.set("popups", json!(true));
        c.toasts.offscreen = true; // (pop-ups shown off the screen: the user may be at the PC)
        was
    });

    // Facebook: rows that link to their conversation
    view_js("facebook", &list(true));
    let fb = read("facebook");
    // the pop-up it makes, and its click
    let card = fb.clone().map(|l| {
        on(move |c| {
            c.counts.insert("facebook".into(), 2);
            c.count_popup_for_test("facebook", Some(l));
        });
        wait(1200);
        page_js("toasts", "JSON.stringify([...document.querySelectorAll('.card:not(.leaving)')].map((c) => [c.querySelector('.title')?.textContent, c.querySelector('.body')?.textContent, !!c.querySelector('img')]))")
    });
    on(|c| c.open_conversation("facebook", c.test_popup_target("facebook")));
    wait(600);
    let fb_opened = view_js("facebook", "String(window.__cdOpened)");
    // the edge tab on it with something unread: the newest unread conversation
    view_js("facebook", "window.__cdOpened = ''; 1");
    on(|c| c.open_conversation("facebook", None));
    wait(600);
    let fb_newest = view_js("facebook", "String(window.__cdOpened)");
    on(|c| {
        c.toasts_dismiss_all();
        c.counts.insert("facebook".into(), 0);
    });

    // Instagram: rows that are buttons with a picture (no links)
    view_js("instagram", &list(false));
    let ig = read("instagram");
    on(move |c| {
        let target = Some(crate::inbox::Latest { name: "Beam".into(), ..Default::default() });
        c.open_conversation("instagram", target);
    });
    wait(600);
    let ig_by_name = view_js("instagram", "String(window.__cdOpened)");

    // the way back to the list: on a conversation, then while something is being typed
    view_js("facebook", "history.pushState({}, '', '/messages/t/2/'); window.__cdBack = false; 1");
    let back = run_back("facebook");
    view_js(
        "facebook",
        "history.pushState({}, '', '/messages/t/3/'); window.__cdBack = false; const b = document.createElement('div'); b.contentEditable = 'true'; b.textContent = 'half a message'; document.getElementById('cd-list').append(b); 1",
    );
    let typing = run_back("facebook");
    on(move |c| {
        c.settings.set("popups", popups_were);
        c.toasts.offscreen = false;
    });
    log!(
        "inbox: Facebook's list read {:?} | pop-up {} | its click opened {fb_opened} | the tab opened {fb_newest} | Instagram's read {:?} | opened by name {ig_by_name} | back to the list {back}, while typing {typing} (expect Nok + her text + link /messages/t/2/ | [Nok, her text, picture] | Nok | Nok | Nok, no link | Beam | back true, typing false)",
        fb.map(|l| (l.name, l.text, l.href)),
        card.unwrap_or_default(),
        ig.map(|l| (l.name, l.text, l.href)),
    );
}

/// inbox.rs's way back to the list, on its own (it normally waits until the chat has been hidden a while).
fn run_back(app: &'static str) -> String {
    on(move |c| c.back_to_list_now(app));
    wait(600);
    format!("{}, {}", view_js(app, "String(window.__cdBack)"), view_js(app, "location.pathname"))
}

/// 1.7.3's fixes from the code review, checked without taking the focus (the panel is shown off
/// screen, like ui171_test): a window opened without a click, the tray menu's "Hide" once the chat
/// already hid, switching off the app on screen, a page that keeps crashing, a pop-up made before
/// its page was ready, a call while all chat pages are muted, the zoom keys after Ctrl+wheel, the
/// update button in a crowded header, and What's new staying at the top while its demos play.
fn fixes173_test() {
    // 1. window.open() without a click: nothing opens (X's view on example.com). A script run by
    // ExecuteScript counts as a click for 5 s (the page's "user activation"), so it opens later.
    on(|c| c.chats.navigate("x", "https://example.com/"));
    wait(4000);
    view_js(
        "x",
        "window.__cdOpened = 'waiting'; setTimeout(() => { window.__cdOpened = String(window.open('about:blank') === null); }, 6000); 1",
    );
    wait(7000);
    let blocked = view_js("x", "String(window.__cdOpened)");
    // 2. the tray menu's "Hide" when the chat already hid by itself: it stays hidden
    let hide = on(|c| {
        c.on_menu("hide");
        c.panel_state.as_str()
    });
    // 3. switching off the app on screen (Settings open): the next one wakes up
    let woke = on(|c| {
        c.sleep_app("facebook");
        let slept = *c.asleep.get("facebook").unwrap_or(&false);
        c.set_active("instagram", false);
        let (state, settings) = (c.panel_state, c.settings_mode);
        c.panel_state = PanelState::Open; // (nothing is shown: only the state)
        c.settings_mode = true;
        c.set_app_enabled("instagram", false);
        let r = (slept, c.active(), *c.asleep.get("facebook").unwrap_or(&false));
        c.set_app_enabled("instagram", true);
        c.set_active("instagram", false);
        c.panel_state = state;
        c.settings_mode = settings;
        r
    });
    // 4. a page that keeps crashing: the third time in 2 minutes it shows "Try again"
    let crashes = on(|c| {
        c.renderer_crashed("x");
        c.renderer_crashed("x");
        let before = c.load_state.get("x").copied();
        c.renderer_crashed("x");
        (before, c.load_state.get("x").copied(), c.retry_timers.contains_key("x"))
    });
    on(|c| c.reload_app("x", false)); // (back to its page)
                                      // 5. a pop-up made before its page was ready: its time starts once the page is
    let early = on(|c| {
        c.ready.remove("toasts");
        c.notice("ChatDock", "self-test");
        let waiting = c.toasts_unstarted();
        c.ready.insert("toasts".into());
        c.toasts_ready();
        let after = c.toasts_unstarted();
        c.toasts_dismiss_all();
        (waiting, after)
    });
    // 6. all chat pages muted, then a call on Discord: the call stays audible
    on(|c| c.wake_app("discord"));
    wait(3000);
    let call = on(|c| {
        let was = c.settings.bool("muted");
        c.settings.set("muted", json!(true));
        c.apply_audio("discord");
        let muted = c.chats.is_muted("discord");
        c.on_call("discord", true, true, false);
        let in_call = c.chats.is_muted("discord");
        c.on_call("discord", false, false, false);
        let after = c.chats.is_muted("discord");
        c.settings.set("muted", json!(was));
        c.apply_audio("discord");
        (muted, in_call, after)
    });
    // 7. the zoom keys after Ctrl+wheel left the zoom between steps (175 %, 50 %)
    let zoom = on(|c| {
        let was = c.zoom_of("x");
        c.settings.set_in("zoom", "x", json!(1.75));
        c.zoom_step("x", -1);
        let down = c.zoom_of("x");
        c.settings.set_in("zoom", "x", json!(0.5));
        c.zoom_step("x", 1);
        let up = c.zoom_of("x");
        c.settings.set_in("zoom", "x", json!(was));
        c.chats.set_zoom("x", was);
        (down, up)
    });
    // 8. the update button in a crowded header (Instagram's width, four apps): one line in its
    // pill; with the volume bar open its % and close button stay clickable
    on(|c| {
        c.upd.status = "available";
        c.upd.version = "1.7.9".into();
        c.set_active("instagram", false);
        let d = c.target_display();
        let g = c.panel_geometry(&d);
        win32::set_bounds(c.panel.hwnd, win32::Rect { x: c.hidden_x(&g, &d), ..g });
        win32::show_inactive(c.panel.hwnd);
        c.panel_state = PanelState::Open;
        c.broadcast_state();
    });
    wait(1200);
    let chip = page_js(
        "panel",
        "(() => { const c = document.getElementById('update'), l = c.querySelector('.label'); return JSON.stringify({ shown: !c.hidden, lines: l.getClientRects().length, fits: c.scrollHeight <= c.clientHeight + 1, width: Math.round(c.getBoundingClientRect().width), text: l.textContent }); })()",
    );
    shot("95-update-chip");
    js_click("panel", "#vol");
    wait(400);
    let bar = page_js(
        "panel",
        "(() => { const hit = (id) => { const r = document.getElementById(id).getBoundingClientRect(); const e = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return !!e && !!e.closest('#' + id); }; return JSON.stringify({ close: hit('vol-done'), percent: hit('vol-num') }); })()",
    );
    shot("96-update-chip-volume");
    js_click("panel", "#vol-done");
    on(|c| {
        c.panel_state = PanelState::Hidden;
        win32::hide(c.panel.hwnd);
        c.upd.status = "idle";
        c.upd.version.clear();
        c.broadcast_state();
    });
    // 9. What's new from 1.6.2 (long, with demos): opens at the top and stays there while they play
    let now = rt::version();
    on(move |c| {
        c.settings.set("whatsNew", json!({ "version": now, "from": "1.6.2", "notes": "", "at": rt::epoch_ms() }));
        c.announce_updated();
    });
    // (a person at the PC may scroll it with the wheel: only the page's own scrolling counts)
    wait(700);
    let top = page_js(
        "whatsnew",
        "(() => { window.__cdScrolls = 0; const siv = Element.prototype.scrollIntoView; Element.prototype.scrollIntoView = function (...a) { window.__cdScrolls++; return siv.apply(this, a); }; return String(document.getElementById('notes').scrollTop); })()",
    );
    wait(7000); // the first demo turn comes after 6 s
    let later = page_js("whatsnew", "String(window.__cdScrolls)");
    shot("97-whats-new-top");
    js_click("whatsnew", "#ok");
    wait(800);
    log!(
        "1.7.3 fixes: window.open without a click gives null {blocked} | tray Hide while hidden -> {hide} | switch off the app on screen (facebook asleep, now active, asleep after) {woke:?} | crashes (before the 3rd, after, retry set) {crashes:?} | pop-up made before its page (unstarted, after ready) {early:?} | muted pages and a call (before, in the call, after) {call:?} | zoom keys from 175 % down, from 50 % up {zoom:?} | update chip {chip} | volume bar {bar} | What's new at the top {top}, scrolled by itself {later} times (expect true | hidden | (true, \"facebook\", false) | (Some(\"ready\"), Some(\"error\"), true) | (1, 0) | (true, false, true) | (1.5, 0.67) | shown, 1 line, fits | close + percent true | 0, 0)"
    );
}

/// 1.7.2: clearing an app's data forgets what it had seen, the edge's watch goes on after an update
/// install that didn't start (the real install_failed), and "the hotkey is taken" names the page
/// the hotkey is on. GolfZzz's own checks are in counts_test, edge_test and review_fixes_test.
/// Nothing shows on the screen.
/// Privacy and safety fixes (nothing takes focus): "Hide from screenshots & streams" on every
/// window of ChatDock's own, including one made later (the monitor cards); Clear data taking the old
/// Electron logins and Discord's server list along; the account name kept out of the log.
fn security_test() {
    // 0. a long session's log: past 1 MB it starts a new file and keeps the last one (first, so
    // the lines below land in the new file)
    let path = log::path().cloned().unwrap_or_default();
    let size = |p: &std::path::Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let filler = "x".repeat(100);
    let mut lines = 0;
    while lines < 20_000 {
        let before = size(&path);
        log!("security: filler {lines} {filler}");
        lines += 1;
        if size(&path) < before {
            break; // a new file
        }
    }
    let (old_size, new_size) = (size(&path.with_extension("log.old")), size(&path));
    log!(
        "security: the log past 1 MB: after {lines} lines the last one kept {} ({} KB), the new one {} KB (expect true (at most 1024 KB), under 10 KB)",
        old_size > 0,
        old_size / 1024,
        new_size / 1024
    );

    // 1. capture: the setting on before the cards exist, so they get it as they are made
    let affinities = || {
        on(|c| {
            let mut all = vec![
                ("panel", c.panel.hwnd),
                ("toasts", c.toastwin.hwnd),
                ("tab", c.tab.hwnd),
                ("glow", c.glow.hwnd),
                ("edge", c.edgewin.hwnd),
            ];
            all.extend(c.identify_hwnds().into_iter().map(|h| ("card", h)));
            all.iter()
                .map(|(name, h)| {
                    let mut a = 0u32;
                    let _ = unsafe { GetWindowDisplayAffinity(win32::h(*h), &mut a) };
                    format!("{name} {a}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
    };
    on(|c| {
        c.set_pref("hideFromCapture", json!(true));
        c.settings_mode = true; // (the cards live while Settings is open)
        c.identify_all();
    });
    wait(2500); // the cards' windows are made off the main thread
    let hidden = affinities();
    on(|c| {
        c.set_pref("hideFromCapture", json!(false));
    });
    let shown = affinities();
    on(|c| {
        c.identify_close();
        c.settings_mode = false;
    });
    log!("security: hidden from capture: {hidden} | switched off: {shown} (expect every window 17 with at least one card, then every window 0)");

    // 2. a page's flood: 40 passkey reports (half with text of the page's own in them) and one
    // too long to read: some lines but at most 30, no page text, nothing of the long one (X's page:
    // Discord's lines are counted below)
    view_js(
        "x",
        "(() => { for (let i = 0; i < 40; i++) window.chrome.webview.postMessage(JSON.stringify({ type: 'passkey', kind: i % 2 ? 'get' : 'INJECTED\\nline', origin: location.origin, mediation: i % 2 ? 'conditional' : 'x INJECTED' })); \
         window.chrome.webview.postMessage(JSON.stringify({ type: 'passkey', kind: 'create', origin: 'https://big.example/', pad: 'y'.repeat(5000) })); return 1; })()",
    );
    wait(1500);
    let text = log::path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let lines = text.lines().filter(|l| l.contains("passkey request blocked x")).count();
    let page_text = text.lines().any(|l| l.contains("INJECTED"));
    let long_one = text.contains("big.example");
    log!("security: a page's flood: {lines} passkey lines, page text in the log {page_text}, the long one read {long_one} (expect 1 to 30, false, false)");

    // 3. this PC (before Clear data reloads the chats)
    this_pc_check();

    // 4. two navigations of ChatDock's own at once (its home, then another page): the chat shows the
    // last, and neither is taken for a link (WebView2 says both were the user's)
    let taken = || {
        log::path().and_then(|p| std::fs::read_to_string(p).ok()).map_or(0, |l| l.matches("discord: (a self-test opens nothing").count())
    };
    let before = taken();
    on(|c| c.load_home("discord"));
    on(|c| c.chats.navigate("discord", "https://example.com/")); // (a moment later, as two steps do)
    let mut host = String::new();
    for _ in 0..40 {
        wait(250);
        host = view_js("discord", "location.host");
        if host == "\"example.com\"" {
            break;
        }
    }
    let taken_for_links = taken() - before;
    on(|c| c.load_home("discord"));
    wait(1500);
    log!("security: two navigations of ChatDock's own at once: the chat shows {host}, taken for links {taken_for_links} (expect \"example.com\", 0)");

    // 5. the windows the pages open (before Clear data reloads the chats)
    popups_check();

    // 6. Clear data: an old Electron login (and one of an app no longer listed), Discord's servers
    let dir = on(|c| c.args.data_dir.clone());
    for app in ["discord", "retired-app"] {
        let _ = std::fs::create_dir_all(dir.join("Partitions").join(app).join("Network"));
    }
    let _ = std::fs::write(dir.join("Local State"), "{}");
    let servers = on(|c| {
        c.settings.set("discordServers", json!({ "Test Server": true }));
        c.clear_app_data("discord");
        c.settings.get("discordServers").to_string()
    });
    let (discord_old, other_old) = (dir.join("Partitions").join("discord").exists(), dir.join("Partitions").join("retired-app").exists());
    on(|c| c.settings_action_test("clear-all", serde_json::Value::Null));
    let (partitions, old_key) = (dir.join("Partitions").exists(), dir.join("Local State").exists());
    log!(
        "security: Clear data (Discord): its old login still there {discord_old}, another's still there {other_old}, servers {servers} | Clear all data: Partitions {partitions}, old key {old_key} (expect false, true, {{}} | false, false)"
    );

    // 7. the log: a path under the Windows account, plain and as {:?} writes it
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    log!("security: paths {home}\\AppData and {:?}", format!("{home}\\AppData"));
    let tail = log::path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let last = tail.lines().rev().find(|l| l.contains("security: paths")).unwrap_or("").to_string();
    let clean =
        !home.is_empty() && !last.to_ascii_lowercase().contains(&home.to_ascii_lowercase()) && last.matches("%USERPROFILE%").count() == 2;
    log!("security: the account name kept out of the log {clean} (expect true)");
}

/// This PC: the WebSockets a chat page opens to it go nowhere; its requests to it, however the
/// address is written and from its workers too, are refused. The requests come from a page with no
/// rules of its own (example.com), so nothing but ChatDock stops them.
fn this_pc_check() {
    let ws = view_js(
        "discord",
        "(() => { const urls = ['ws://127.1:6463/', 'ws://x.localhost:6463/', 'ws://[::1]:6463/'].map((u) => { const s = new WebSocket(u); s.close(); return s.url; }); \
         const f = document.createElement('iframe'); document.documentElement.appendChild(f); \
         const inFrame = String(f.contentWindow.WebSocket).includes('[native code]') ? 'native' : 'wrapped'; f.remove(); \
         return urls.join(' ') + ' | in a new frame ' + inFrame; })()",
    );
    let refused = || crate::chats::LOCAL_BLOCKED.load(std::sync::atomic::Ordering::Relaxed);
    let earlier = refused();
    on(|c| c.chats.navigate("discord", "https://example.com/"));
    let mut host = String::new();
    for _ in 0..40 {
        wait(250);
        host = view_js("discord", "location.host");
        if host == "\"example.com\"" {
            break;
        }
    }
    wait(500); // (its own scripts run)
    let before = refused();
    let page = view_js_async(
        "discord",
        "Promise.all(['https://127.0.0.2:8443/', 'https://x.localhost/', 'https://2130706433/', 'https://[::ffff:127.0.0.1]/', 'https://localhost./'] \
         .map((u) => fetch(u, { mode: 'no-cors' }).then(() => 'answered', () => 'failed'))).then((r) => r.join(' '))",
    );
    let worker = view_js_async(
        "discord",
        "new Promise((ok) => { const src = URL.createObjectURL(new Blob([\"fetch('https://127.0.0.3/', { mode: 'no-cors' }).then(() => postMessage('answered'), () => postMessage('failed'))\"], { type: 'text/javascript' })); \
         const w = new Worker(src); w.onmessage = (e) => ok(e.data); w.onerror = () => ok('no worker'); })",
    );
    let shared = view_js_async(
        "discord",
        "new Promise((ok) => { const src = URL.createObjectURL(new Blob([\"onconnect = (e) => { const port = e.ports[0]; fetch('https://127.0.0.4/', { mode: 'no-cors' }).then(() => port.postMessage('answered'), () => port.postMessage('failed')); }\"], { type: 'text/javascript' })); \
         const w = new SharedWorker(src); w.port.onmessage = (e) => ok(e.data); w.onerror = () => ok('no worker'); w.port.start(); })",
    );
    wait(500);
    let blocked = refused() - before;
    on(|c| c.load_home("discord"));
    log!(
        "security: this PC: WebSockets to {ws} | refused while the chats loaded {earlier} | from {host}: fetches {page}, a worker's {worker}, a shared worker's {shared}: refused {blocked} (expect wss://local-blocked.invalid/ three times | in a new frame wrapped | from \"example.com\": answered five times, answered, answered: refused 7)"
    );
}

/// A window a chat page opens itself (a blank one here, from a page with no rules of its own) is
/// ChatDock's: the page script, the local-access block and hiding from capture work there, and the
/// site still has it (opener, close()). An address that isn't the app's never loads in a blank one.
/// A call link's window is the app's call until it closes.
fn popups_check() {
    let refused = || crate::chats::LOCAL_BLOCKED.load(std::sync::atomic::Ordering::Relaxed);
    // new windows on screen, ChatDock's or the chats' browser's
    let windows = || {
        on(|_| {
            let mut all = win32::top_windows_of(std::process::id());
            all.extend(win32::top_windows_of(crate::chats::browser_pid()));
            all.into_iter().filter(|&h| win32::is_visible(h) && win32::window_rect(h).w >= 200).collect::<Vec<_>>()
        })
    };
    let lines = |text: &str| log::path().and_then(|p| std::fs::read_to_string(p).ok()).map_or(0, |l| l.matches(text).count());
    on(|c| c.chats.navigate("discord", "https://example.com/"));
    for _ in 0..40 {
        wait(250);
        if view_js("discord", "location.host") == "\"example.com\"" {
            break;
        }
    }
    wait(500);
    on(|c| c.set_pref("hideFromCapture", json!(true)));

    // a blank one
    let before = windows();
    let blocked_before = refused();
    let opened =
        view_js("discord", "(() => { window.__w = window.open('about:blank', '_blank', 'width=480,height=360'); return !!window.__w; })()");
    wait(2500);
    let w = windows().into_iter().find(|h| !before.contains(h)).unwrap_or(0);
    let ours = w != 0 && win32::window_pid(w) == std::process::id();
    let mut affinity = 0u32;
    let _ = unsafe { GetWindowDisplayAffinity(win32::h(w), &mut affinity) };
    let inside = view_js(
        "discord",
        "(() => { const w = window.__w; return (w.opener === window) + ' ' + (String(w.WebSocket).includes('[native code]') ? 'native' : 'wrapped'); })()",
    );
    view_js("discord", "window.__w.fetch('https://127.0.0.5/', { mode: 'no-cors' }).catch(() => 0); 1");
    wait(800);
    let blocked = refused() - blocked_before;
    view_js("discord", "window.__w.close(); 1");
    wait(1000);
    let closed = w != 0 && !win32::is_window(w);

    // a blank one the site then sends to another site
    let before = windows();
    let decided = || {
        lines("discord: a web page opens outside ChatDock (from a window of its own)")
            + lines("discord: a web page not opened (nobody clicked it, in a window of its own)")
    };
    let said = decided();
    view_js(
        "discord",
        "(() => { const w = window.open('about:blank'); window.__w2 = w; setTimeout(() => { try { w.location = 'https://example.org/'; } catch (e) {} }, 400); return 1; })()",
    );
    wait(3000);
    let gone = windows().into_iter().all(|h| before.contains(&h));
    let logged = decided() > said;
    view_js("discord", "(() => { try { window.__w2.close(); } catch (e) {} return 1; })()");

    // a call link
    let icons = || on(|c| c.ui_state()["calls"].to_string());
    let before = windows();
    view_js("instagram", "(() => { window.__c = window.open('https://www.instagram.com/call/?selftest=1'); return 1; })()");
    wait(3500);
    let call = windows().into_iter().find(|h| !before.contains(h)).unwrap_or(0);
    let call_ours = call != 0 && win32::window_pid(call) == std::process::id();
    let in_call = icons();
    // closed as you would (its page may have cut the tie to the chat that opened it)
    on(move |_| unsafe {
        let _ = PostMessageW(Some(win32::h(call)), WM_CLOSE, WPARAM(0), LPARAM(0));
    });
    wait(2000);
    let after_call = icons();
    on(|c| c.set_pref("hideFromCapture", json!(false)));
    on(|c| c.load_home("discord"));
    log!(
        "security: windows the pages open: a blank one opened {opened}, ChatDock's {ours} (capture {affinity}) | in it: {inside} | refused {blocked} | closed by the site {closed} | one sent to another site: gone {gone}, the link decided on {logged} | a call link's window ChatDock's {call_ours}, icons {in_call}, after it closed {after_call} (expect true, true (17) | \"true wrapped\" | 1 | true | true, true | true, [instagram call], [])"
    );
}

fn fixes172_test() {
    // 1. seen counts: a new login starts with nothing seen (and no notification from it yet)
    let cleared = on(|c| {
        c.site_counts.insert("x".into(), 5);
        c.settings.set_in("seenCounts", "x", json!(5));
        c.last_content_at.insert("x".into(), rt::epoch_ms());
        c.clear_app_data("x");
        (c.settings.get("seenCounts").get("x").cloned(), c.site_counts.get("x").copied(), c.last_content_at.get("x").copied())
    });
    // 2. the edge's watch: running, idle while quitting for the installer, running again when the
    // install didn't start
    let looks = || on(|c| c.edge.looks);
    let a = looks();
    wait(600);
    let running = looks() > a;
    on(|c| {
        c.upd.status = "installing";
        c.quitting = true;
    });
    wait(400);
    let b = looks();
    wait(600);
    let idle = looks() == b;
    on(|c| c.install_failed("self-test"));
    let d = looks();
    wait(600);
    let again = looks() > d;
    // 3. the text
    let text = on(|c| c.tv("balloon.hotkeyBody", &[("hotkey", "Ctrl+Alt+C".into())]));
    log!(
        "1.7.2 fixes: after clearing X's data seen, site, last notification {cleared:?} | the edge's watch running {running}, idle while installing {idle}, back after the failed install {again} | {text} (expect (Some(0), Some(0), None), true, true, true, ... Opening the chat)"
    );
}

/// Which protected-media (DRM) systems the chats' browser offers: Spotify's web player needs
/// Widevine to play anything.
fn drm_test() {
    let probe = |system: &str| {
        view_js_async(
            "discord",
            &format!(
                "navigator.requestMediaKeySystemAccess('{system}', [{{ initDataTypes: ['cenc'], audioCapabilities: [{{ contentType: 'audio/mp4; codecs=\"mp4a.40.2\"' }}], \
                 videoCapabilities: [{{ contentType: 'video/mp4; codecs=\"avc1.42E01E\"' }}] }}]).then((a) => 'yes: ' + a.keySystem + ' ' + JSON.stringify(a.getConfiguration().audioCapabilities), (e) => 'no: ' + e.name + ' ' + e.message)"
            ),
        )
    };
    log!(
        "drm: widevine {} | playready {} | clearkey {}",
        probe("com.widevine.alpha"),
        probe("com.microsoft.playready.recommendation"),
        probe("org.w3.clearkey")
    );
    log!("drm: browser {} | user agent {}", crate::chats::browser_version(), view_js("discord", "navigator.userAgent"));
}

/// What keeps running while ChatDock sits hidden: animations left running in its own pages (each
/// one redraws on every screen refresh, 300 times a second on a 300 Hz screen), then a quiet
/// stretch for measuring CPU from outside (scratchpad perf-sample.ps1).
fn perf_test() {
    wait(8000); // the pages load
    for label in ["panel", "tab", "glow", "edge", "toasts"] {
        let running = page_js(
            label,
            "JSON.stringify(document.getAnimations().filter((a) => a.playState === 'running').map((a) => { \
             const t = a.effect && a.effect.target; \
             return (a.animationName || a.constructor.name) + ' on ' + (t ? (t.id ? '#' + t.id : t.className || t.tagName) : '?') + ((a.effect && a.effect.pseudoElement) || ''); }))",
        );
        log!("animations running in {label}: {running}");
    }
    // A: as it is; B: like before 1.7, animations allowed while hidden (measure both from outside)
    log!("perf: A (as it is) for 22 s");
    wait(22000);
    log!(
        "perf: B (animations allowed while hidden, like before; running now: {}) for 22 s",
        page_js(
            "panel",
            "document.body.classList.remove('panel-hidden'), document.getAnimations().filter((a) => a.playState === 'running').map((a) => a.animationName).join(' ')"
        )
    );
    wait(22000);
    log!("perf: done");
}

/// A DevTools protocol call in an app's page; its JSON answer.
fn cdp_call(id: &'static str, method: &'static str, params: String) -> String {
    let (tx, rx) = mpsc::channel::<String>();
    later(move |c| {
        c.chats.cdp(id, method, &params, move |r| {
            let _ = tx.send(r);
        })
    });
    rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| "err:no answer".into())
}

/// The visible top-level windows of these programs: program, class, title, owner's class, place.
fn windows_of(exes: &[&str]) -> Vec<String> {
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM, TRUE},
            UI::WindowsAndMessaging::{EnumWindows, GetWindow, GetWindowTextW, IsWindowVisible, GW_OWNER},
        },
    };
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        let list = &mut *(l.0 as *mut Vec<isize>);
        if IsWindowVisible(h).as_bool() {
            list.push(h.0 as isize);
        }
        TRUE
    }
    let mut all: Vec<isize> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut Vec<isize> as isize));
    }
    all.into_iter()
        .filter_map(|h| {
            let exe = win32::process_name(h);
            if !exes.iter().any(|e| exe.eq_ignore_ascii_case(e)) {
                return None;
            }
            let mut buf = [0u16; 200];
            let n = unsafe { GetWindowTextW(HWND(h as _), &mut buf) }.max(0) as usize;
            let owner = unsafe { GetWindow(HWND(h as _), GW_OWNER) }.map(|o| o.0 as isize).unwrap_or(0);
            Some(format!(
                "{exe} {} {:?} owner {} {:?}",
                win32::class_name(h),
                String::from_utf16_lossy(&buf[..n]),
                if owner != 0 { win32::class_name(owner) } else { "-".into() },
                win32::window_rect(h)
            ))
        })
        .collect()
}

fn click_at(x: i32, y: i32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
    };
    let was = win32::cursor_pos();
    set_cursor(x, y);
    wait(60);
    let click = |flags| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } } };
    unsafe {
        SendInput(&[click(MOUSEEVENTF_LEFTDOWN), click(MOUSEEVENTF_LEFTUP)], std::mem::size_of::<INPUT>() as i32);
    }
    wait(60);
    set_cursor(was.0, was.1);
}

/// Screen sharing from a chat site (Discord's "Share your screen" calls getDisplayMedia): WebView2
/// shows its own picker inside the panel. The test picks the entire screen, sends the stream through
/// a loopback connection inside the page (nothing leaves the PC) and counts the frames that arrive,
/// with the panel out and after it has hidden. CHATDOCK_SHARE_CLICKS="tabX,tabY;itemX,itemY;shareX,shareY"
/// (panel-relative px) says where to click in the picker; without it the test only shows the picker.
fn share_test() {
    wait(7000); // the pages load
    let app = on(|c| if c.is_enabled("discord") { "discord" } else { c.enabled_apps()[0] });
    log!("share: WebView2 runtime {} | app {app}", on(|_| crate::chats::browser_version()));
    on(move |c| c.open_panel(Some(app), "selftest"));
    wait(1500);
    let js = r#"
      window.__share = 'pending'; window.__frames = [];
      navigator.mediaDevices.getDisplayMedia({ video: { frameRate: 30 }, audio: true }).then(async (s) => {
        window.__share = 'ok: ' + s.getTracks().map(t => t.kind + '=' + t.label).join(', ');
        const a = new RTCPeerConnection(), b = new RTCPeerConnection();
        a.onicecandidate = e => e.candidate && b.addIceCandidate(e.candidate);
        b.onicecandidate = e => e.candidate && a.addIceCandidate(e.candidate);
        s.getTracks().forEach(t => a.addTrack(t, s));
        await a.setLocalDescription(await a.createOffer()); await b.setRemoteDescription(a.localDescription);
        await b.setLocalDescription(await b.createAnswer()); await a.setRemoteDescription(b.localDescription);
        window.__track = s.getVideoTracks()[0];
        setInterval(async () => {
          const st = await b.getStats(); let n = 0;
          st.forEach(r => { if (r.type === 'inbound-rtp' && r.kind === 'video') n = r.framesDecoded || 0; });
          window.__frames.push([Date.now(), n, document.visibilityState, window.__track.readyState]);
        }, 1000);
      }, e => { window.__share = 'err: ' + e.name + ': ' + e.message; });
      'asked'"#;
    let asked = cdp_call(app, "Runtime.evaluate", json!({ "expression": js, "userGesture": true }).to_string());
    wait(2000);
    let panel = on(|c| win32::window_rect(c.panel.hwnd));
    screen_grab("71-picker", panel, (0, 0, 0));
    let clicks: Vec<(i32, i32)> = std::env::var("CHATDOCK_SHARE_CLICKS")
        .unwrap_or_default()
        .split(';')
        .filter_map(|p| {
            let (x, y) = p.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect();
    for (i, (x, y)) in clicks.iter().enumerate() {
        click_at(panel.x + x, panel.y + y);
        wait(900);
        screen_grab(&format!("72-picker-{i}"), panel, (0, 0, 0));
    }
    wait(2500);
    let fg = win32::foreground_window();
    log!(
        "share: asked {asked} | page says {} | panel {:?} | in front: {} {} | WebView2 windows: {:?}",
        view_js(app, "window.__share"),
        panel_state(),
        win32::process_name(fg),
        win32::class_name(fg),
        windows_of(&["msedgewebview2.exe"])
    );
    let r = win32::window_rect(fg);
    if win32::process_name(fg).eq_ignore_ascii_case("msedgewebview2.exe") {
        screen_grab("73-share-front", r, (0, 0, 0));
    }
    if clicks.is_empty() {
        log!("share: no clicks given: leaving the picker as it is");
    } else {
        wait(3000);
        let open = view_js(app, "JSON.stringify(window.__frames.slice(-3))");
        close_panel(); // the chat hides (its page goes invisible) while the share goes on
        wait(6000);
        let hidden = view_js(app, "JSON.stringify(window.__frames.slice(-4))");
        log!("share: frames with the panel out {open} | after it hid {hidden} (expect the count to keep rising, track live)");
        view_js(app, "window.__track && window.__track.stop(); 1");
    }
    close_panel();
}

/// The newest release before the running one that has a What's new text (updating from it lists
/// just this release).
fn previous_release() -> String {
    let now = rt::version();
    i18n::news_keys()
        .into_iter()
        .map(|(v, _)| v)
        .filter(|v| crate::core::version_less(v, &now))
        .max_by_key(|v| crate::core::version_parts(v))
        .unwrap_or(now)
}

fn toasts_snapshot() -> String {
    on(|c| json!({ "visible": c.toasts_visible(), "count": c.toasts_count() }).to_string())
}

fn panel_state() -> PanelState {
    on(|c| c.panel_state)
}

fn close_panel() {
    on(|c| {
        if c.panel_state != PanelState::Hidden {
            c.close_panel(true, "selftest");
        }
    });
    wait(600);
}

/// Frames the frame clock delivers in half a second (the screen's refresh rate with vertical blanks).
fn frame_clock() -> (u32, u32, &'static str) {
    fn step(n: u32, t0: f64, tx: mpsc::Sender<u32>) {
        frames::request(move |now| {
            if now - t0 < 500.0 {
                step(n + 1, t0, tx);
            } else {
                let _ = tx.send(((n + 1) as f64 / ((now - t0) / 1000.0)).round() as u32);
            }
        });
    }
    let (tx, rx) = mpsc::channel();
    later(move |_| step(0, rt::now_ms(), tx));
    let fps = rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0);
    let (hz, mode) = frames::info();
    (fps, hz, mode)
}

// ---------------------------------------------------------------------------------------------
// The edge: frame clock, the tab following the cursor, holding on the edge, pop-ups sliding away.
// Only the panel slide (with_panel) takes focus.
// ---------------------------------------------------------------------------------------------
fn edge_test(with_panel: bool) {
    let (fps, hz, mode) = frame_clock();
    log!("frame clock: fps {fps} | hz {hz} | mode {mode} (expect fps close to hz, mode vblank)");
    if with_panel {
        on(|c| {
            c.anim_frames = 0;
            c.open_panel(None, "selftest");
        });
        wait(260 + 150);
        let frames = on(|c| c.anim_frames);
        log!("panel slide-in: {frames} frames in 260 ms = {} fps (expect close to {hz})", (frames as f64 / 0.26).round());
        close_panel();
    }

    // A pretend cursor slides 300 px down the edge in 0.3 s: the tab must follow in small steps.
    let (edge_x, y0, height) = on(|c| {
        let b = c.target_display().bounds;
        let x = if c.on_left() { b.x } else { b.right() - 1 };
        (x, b.y + b.h / 2 - 150, b.h)
    });
    on(move |c| {
        c.edge.test_cursor = Some((edge_x, y0, false));
        c.show_tab(Some(y0));
    });
    wait(250);
    let (tx, rx) = mpsc::channel::<(u32, i32)>();
    fn follow(t0: f64, prev: i32, moves: u32, biggest: i32, edge_x: i32, y0: i32, tx: mpsc::Sender<(u32, i32)>) {
        frames::request(move |now| {
            let y = crate::core::with(|c| {
                c.edge.test_cursor = Some((edge_x, y0 + (300.0 * ((now - t0) / 300.0).min(1.0)).round() as i32, false));
                c.tab_bounds().y
            })
            .unwrap_or(prev);
            let (moves, biggest) = if y != prev { (moves + 1, biggest.max((y - prev).abs())) } else { (moves, biggest) };
            if now - t0 < 650.0 {
                follow(t0, y, moves, biggest, edge_x, y0, tx);
            } else {
                let _ = tx.send((moves, biggest));
            }
        });
    }
    later(move |c| {
        let prev = c.tab_bounds().y;
        follow(rt::now_ms(), prev, 0, 0, edge_x, y0, tx);
    });
    let (moves, biggest) = rx.recv_timeout(Duration::from_secs(5)).unwrap_or((0, 0));
    let inside = on(move |c| {
        let t = c.tab_bounds();
        let s = c.target_display().scale;
        let y = y0 + 300;
        y >= t.y + (16.0 * s) as i32 && y <= t.bottom() - (16.0 * s) as i32
    });
    log!(
        "tab follows the cursor: {moves} moves | biggest step {biggest} px | cursor inside the pill at the end {inside} (expect many moves, <= {} px, true)",
        (1000.0 / hz.max(1) as f64).ceil() as i32 * 2 + 4
    );
    on(|c| {
        c.hide_tab(true);
        c.edge.test_cursor = None;
    });
    wait(300);

    // Holding the cursor on the edge: the line grows, and the tab only comes out when the time is up.
    let hold_was = on(|c| c.settings.get("edgeHold").clone());
    let hy = on(move |c| {
        let b = c.target_display().bounds;
        c.settings.set("edgeHold", json!(1));
        b.y + (height as f64 * 0.4).round() as i32
    });
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false)));
    wait(550);
    let line_state = page_js("edge", "JSON.stringify({ mode, h: H })");
    let (line_mid, tab_mid) = on(|c| (win32::is_visible(c.edgewin.hwnd), c.edge.tab_shown));
    shot("33-hold-line");
    wait(700);
    let tab_late = on(|c| c.edge.tab_shown);
    log!("hold 1 s: at 0.55 s line shown {line_mid} {line_state} | tab {tab_mid} | at 1.25 s tab {tab_late} (expect true {{\"mode\":\"hold\",...}}, false, true)");
    on(move |c| {
        c.hide_tab(true);
        c.edge.test_cursor = Some((edge_x - 300, hy, false));
    });
    wait(500);
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false))); // held half way, then let go
    wait(500);
    on(move |c| c.edge.test_cursor = Some((edge_x - 300, hy, false)));
    wait(60);
    let (tab, line) = on(|c| (c.edge.tab_shown, c.edge.hold_line));
    log!("hold given up half way: tab {tab} | line holding {line} (expect false, false)");
    wait(400);
    let gone = on(|c| !win32::is_visible(c.edgewin.hwnd));
    log!("line window gone after shrinking back: {gone} (expect true)");
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, true))); // a game holds the mouse against the edge
    wait(1300);
    let (line, tab) = on(|c| (c.edge.hold_line, c.edge.tab_shown));
    log!("mouse held by a game: line {line} | tab {tab} (expect false, false)");
    // An update that couldn't start: ChatDock was quitting for the installer, then carries on
    // (install_failed). The edge has to work again.
    let away = edge_x + if on(|c| c.on_left()) { 300 } else { -300 };
    on(move |c| {
        c.edge.test_cursor = Some((away, hy, false));
        c.quitting = true;
    });
    wait(500);
    on(|c| c.quitting = false);
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false)));
    wait(1600);
    let tab = on(|c| c.edge.tab_shown);
    log!("after an update that couldn't start: tab {tab} (expect true)");
    on(move |c| {
        c.hide_tab(true);
        c.edge.test_cursor = Some((away, hy, false));
    });
    wait(300);
    wheel_test(edge_x, hy);
    on(move |c| {
        c.edge.test_cursor = None;
        c.settings.set("edgeHold", hold_was);
    });
    log!(
        "mouse taken by a game right now: {} | pointer hidden {} | confined {}",
        win32::mouse_captured(),
        win32::cursor_hidden(),
        win32::cursor_confined()
    );

    // A pop-up that closes slides away first; the window goes once it is gone.
    on(|c| c.test_popup());
    wait(900);
    log!("pop-up page: {}", page_js("toasts", "JSON.stringify([document.visibilityState, document.querySelectorAll('.card').length])"));
    on(|c| c.toasts_dismiss_all());
    wait(60);
    let leaving = page_js("toasts", "document.querySelectorAll('.card.leaving').length");
    let up_meanwhile = on(|c| c.toasts_visible());
    wait(500);
    let gone = on(|c| !c.toasts_visible());
    wait(600);
    let gone_late = on(|c| !c.toasts_visible());
    log!(
        "pop-up closing: sliding away {leaving} | window still up meanwhile {up_meanwhile} | window gone after {gone} | gone 1.1 s later {gone_late} (expect 1, true, true, true; with the screen locked pages don't animate, so only the last)"
    );
}

/// The mouse wheel turning with the pointer on the edge (scrolling a page's scrollbar there): the
/// line goes, no tab comes, until the pointer has left the edge; holding works again after that, and
/// the wheel on an open tab puts it away. Then a real wheel event (a zero step: nothing scrolls)
/// through Windows, to see the raw input copy arrive.
fn wheel_test(edge_x: i32, hy: i32) {
    let inward = if on(|c| c.on_left()) { 1 } else { -1 };
    on(move |c| {
        c.settings.set("edgeHold", json!(1));
        c.edge.test_cursor = Some((edge_x + inward * 300, hy, false));
    });
    wait(300);
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false)));
    wait(400);
    let (line_before, listening) = on(|c| (c.edge.hold_line, c.edge.dwell_start != 0));
    crate::wheel::test_turn();
    wait(200);
    let (line, scrolling) = on(|c| (c.edge.hold_line, c.edge.scrolling));
    wait(1500);
    let tab = on(|c| c.edge.tab_shown);
    on(move |c| c.edge.test_cursor = Some((edge_x + inward * 30, hy, false)));
    wait(250);
    let still = on(|c| c.edge.scrolling);
    on(move |c| c.edge.test_cursor = Some((edge_x + inward * 300, hy, false)));
    wait(300);
    let freed = on(|c| !c.edge.scrolling);
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false)));
    wait(1400);
    let tab_again = on(|c| c.edge.tab_shown);
    crate::wheel::test_turn();
    wait(250);
    let tab_after = on(|c| c.edge.tab_shown);
    log!(
        "wheel on the edge: holding {line_before} (listening {listening}) | after the wheel: line {line}, scrolling {scrolling} | 1.5 s later tab {tab} | 30 px off still scrolling {still} | 300 px off freed {freed} | back and held: tab {tab_again} | wheel on the tab: tab {tab_after} (expect true (true) | false, true | false | true | true | true | false)"
    );
    on(move |c| c.settings.set("edgeWheel", json!(false)));
    on(move |c| c.edge.test_cursor = Some((edge_x + inward * 300, hy, false)));
    wait(300);
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false)));
    wait(400);
    crate::wheel::test_turn();
    wait(1200);
    log!("wheel with the setting off: tab {} (expect true: it opens as before)", on(|c| c.edge.tab_shown));
    on(move |c| {
        c.settings.set("edgeWheel", json!(true));
        c.hide_tab(true);
        c.edge.test_cursor = Some((edge_x + inward * 300, hy, false));
    });
    wait(300);

    // the real thing: while the edge is held (5 s, so no tab), a wheel event with a zero step
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT};
    on(move |c| {
        c.settings.set("edgeHold", json!(5));
        c.edge.test_cursor = Some((edge_x, hy, false));
    });
    wait(400);
    let before = crate::wheel::last();
    let listening = on(|c| c.edge.dwell_start != 0);
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: MOUSEEVENTF_WHEEL, mouseData: 0, ..Default::default() } },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    wait(300);
    let seen = crate::wheel::last() > before;
    let (scrolling, line) = on(|c| (c.edge.scrolling, c.edge.hold_line));
    log!("wheel through Windows: listening {listening} | sent {sent} | raw input saw it {seen} | edge now scrolling {scrolling}, line {line} (expect true, 1, true, true, false)");
    on(move |c| c.edge.test_cursor = Some((edge_x + inward * 300, hy, false)));
    wait(300);
}

// ---------------------------------------------------------------------------------------------
// Everything else
// ---------------------------------------------------------------------------------------------
fn full_test() {
    wait(11_000); // the chats load
    for id in on(|c| c.enabled_apps()) {
        let info = on(move |c| {
            format!(
                "page {id} | url {} | load {} | firstShown {}",
                c.chats.source(id),
                c.load_state.get(id).copied().unwrap_or("?"),
                c.first_shown.get(id).copied().unwrap_or(false)
            )
        });
        log!("{info} | title {}", view_js(id, "document.title"));
    }
    log!("UA {}", view_js(on(|c| c.enabled_apps()[0]), "navigator.userAgent"));
    let m = on(|c| c.memory_stats());
    log!("RAM after start: {} MB in {} processes | per app {:?}", m.total, m.processes, m.per_app);
    for id in on(|c| c.enabled_apps()) {
        let r = view_js_async(
            id,
            "Promise.all([CredentialsContainer.prototype.get.name, CredentialsContainer.prototype.create.name, \
             window.PublicKeyCredential ? PublicKeyCredential.isConditionalMediationAvailable() : 'n/a', \
             Notification.permission, navigator.permissions.query({ name: 'notifications' }).then((p) => p.state)])",
        );
        log!("site hooks {id} {r} (expect guarded, guarded, false, granted, granted)");
    }

    if !on(|c| c.settings.bool("onboarded")) {
        // first run: finish the welcome screen
        if panel_state() == PanelState::Hidden {
            on(|c| c.open_panel(None, "selftest"));
            wait(600);
        }
        shot("01-welcome");
        on(|c| c.finish_onboarding(None));
        for id in on(|c| c.enabled_apps()) {
            on(move |c| c.set_active(id, true));
            wait(1500);
            shot(&format!("02-app-{id}"));
        }
        let vis: Vec<String> =
            on(|c| c.enabled_apps()).into_iter().map(|id| format!("{id}={}", view_js(id, "document.visibilityState"))).collect();
        log!("visibility (panel open, last app active): {}", vis.join(" "));
        close_panel();
    }
    let (state, visible) = on(|c| (c.panel_state, win32::is_visible(c.panel.hwnd)));
    log!("panel after close: {state:?} | visible {visible}");
    let vis: Vec<String> =
        on(|c| c.enabled_apps()).into_iter().map(|id| format!("{id}={}", view_js(id, "document.visibilityState"))).collect();
    log!("visibility (panel hidden): {} (expect hidden)", vis.join(" "));
    let popups_were = on(|c| c.settings.get("popups").clone());
    on(|c| c.settings.set("popups", json!(false))); // the count checks below shouldn't pop anything up

    let first = on(|c| c.enabled_apps()[0]);
    let second = on(|c| c.enabled_apps().get(1).copied().unwrap_or(c.enabled_apps()[0]));
    on(move |c| c.on_title(first, "(3) Instagram"));
    log!("count after \"(3) …\" = {} (expect 3)", on(move |c| c.counts.get(first).copied().unwrap_or(0)));
    on(move |c| c.on_title(first, "Somchai sent you a message"));
    log!("count right after flash title = {} (expect 3)", on(move |c| c.counts.get(first).copied().unwrap_or(0)));
    wait(3400);
    log!("count 3.4s after a plain title = {} (expect 0)", on(move |c| c.counts.get(first).copied().unwrap_or(0)));
    on(move |c| c.on_title(second, "(2) Facebook"));
    wait(300);
    log!("glow visible = {} {:?} (expect true)", on(|c| win32::is_visible(c.glow.hwnd)), on(|c| win32::window_rect(c.glow.hwnd)));
    shot("04-glow");

    on(|c| {
        let b = c.target_display().bounds;
        c.show_tab(Some(b.y + b.h / 2));
    });
    wait(300);
    log!(
        "tab visible = {} {:?} | glow hidden = {}",
        on(|c| win32::is_visible(c.tab.hwnd)),
        on(|c| c.tab_bounds()),
        on(|c| !win32::is_visible(c.glow.hwnd))
    );
    log!(
        "tab renderer {}",
        page_js("tab", "JSON.stringify({ cls: document.getElementById('pill').className, transform: getComputedStyle(document.getElementById('pill')).transform, visibility: document.visibilityState })")
    );
    shot("05-tab");

    // Click the last app's icon on the tab
    let last = on(|c| *c.enabled_apps().last().unwrap());
    on(move |c| c.set_active(first, false));
    wait(200);
    let clicked = js_click("tab", &format!(".app[data-app=\"{last}\"]"));
    wait(700);
    let (state, active) = on(|c| (c.panel_state, c.active()));
    log!("after clicking the tab {last} icon (clicked {clicked}): panel {state:?} | active {active} (expect Open, {last})");
    if state == PanelState::Hidden {
        on(move |c| c.open_panel(Some(last), "selftest"));
    }
    wait(300);
    log!(
        "panel: {:?} | bounds {:?} | focused {} | tab hidden {}",
        panel_state(),
        on(|c| win32::window_rect(c.panel.hwnd)),
        on(|c| win32::foreground_window() == c.panel.hwnd),
        on(|c| !win32::is_visible(c.tab.hwnd))
    );
    shot("06-open-last-app");
    view_check("06-open-last-app");
    close_panel();
    on(move |c| {
        for id in c.enabled_apps() {
            c.set_count(id, 0);
        }
        c.settings.set("popups", popups_were);
    });

    // Pop-ups: the site raises a real web notification -> ChatDock pop-up -> click -> that chat opens
    let pop_app = on(|c| if c.is_enabled("discord") { "discord" } else { c.enabled_apps()[0] });
    let avatar = format!("data:image/png;base64,{}", crate::toasts::base64(include_bytes!("../../assets/icon.png")));
    let r = view_js(
        pop_app,
        &format!(
            "(() => {{ const n = new Notification('สมชาย ใจดี', {{ body: 'ว่างไหม เย็นนี้ลงแรงค์ด้วยกัน 5 ตาพอ 🎮', icon: {}, tag: 'dm-1' }}); \
             n.onclick = () => {{ window.__cdClicked = (window.__cdClicked || 0) + 1; }}; return Notification.permission; }})()",
            json!(avatar)
        ),
    );
    wait(900);
    log!("pop-up after a site notification ({r}): {} (expect visible, count 1)", toasts_snapshot());
    shot("08-popup");
    let clicked = js_click("toasts", ".card");
    wait(900);
    let (state, active) = on(|c| (c.panel_state, c.active()));
    log!(
        "after clicking the pop-up (clicked {clicked}): panel {state:?} | active {active} | site onclick ran {} | pop-ups left {} (expect Open, {pop_app}, 1, 0)",
        view_js(pop_app, "window.__cdClicked || 0"),
        on(|c| c.toasts_count())
    );
    close_panel();

    // A page that just (re)loaded shows its old unread count: that must not pop up as "new message"
    let quiet = on(|c| if c.is_enabled("x") { "x" } else { c.enabled_apps()[0] });
    on(move |c| {
        c.load_started_at.insert(quiet.to_string(), rt::epoch_ms());
        c.on_title(quiet, "(4) Messages / X");
    });
    wait(2900);
    log!("pop-up right after a page load: {} (expect not visible)", toasts_snapshot());
    on(move |c| c.set_count(quiet, 0));

    // A site that gives no notification text: the unread count + title flash still pop something up
    on(move |c| {
        c.load_started_at.insert(quiet.to_string(), 0);
        c.last_content_at.insert(quiet.to_string(), 0);
        c.on_title(quiet, "Somchai sent you a message");
        c.on_title(quiet, "(1) Messages / X");
    });
    wait(2900);
    log!("pop-up from the unread count: {} (expect visible, count 1)", toasts_snapshot());
    shot("09-popup-fallback");
    on(move |c| c.set_count(quiet, 0));
    wait(600);
    log!("pop-up gone once read elsewhere: {} (expect not visible)", toasts_snapshot());

    // Switching an app on and off at runtime
    on(|c| c.set_app_enabled("telegram", true));
    wait(1500);
    log!(
        "telegram on: view {} | tab height {} | apps {:?}",
        on(|c| c.chats.has("telegram")),
        on(|c| c.tab_bounds().h),
        on(|c| c.enabled_apps())
    );
    on(|c| c.set_app_enabled("telegram", false));
    wait(300);
    log!("telegram off: view {} | apps {:?}", on(|c| c.chats.has("telegram")), on(|c| c.enabled_apps()));
    on(move |c| c.open_panel(Some(first), "selftest"));
    wait(600);

    // Failed load -> error screen -> retry -> recovers
    on(move |c| {
        c.set_active(first, false);
        c.chats.navigate(first, "https://chatdock-selftest.invalid/");
    });
    wait(2500);
    let (load, visible) = on(move |c| (c.load_state.get(first).copied().unwrap_or("?"), c.chats.is_visible(first)));
    log!("failed load: load state = {load} | view visible = {visible} (expect error, false)");
    shot("07-error");
    on(move |c| c.reload_app(first, false)); // what the "Try again" button does
    wait(7000);
    let (load, visible, url) =
        on(move |c| (c.load_state.get(first).copied().unwrap_or("?"), c.chats.is_visible(first), c.chats.source(first)));
    log!("after retry: load state = {load} | view visible = {visible} | url {url} (expect ready, true)");
    close_panel();
    log!(
        "panel after close: {:?} | visible {} | glow visible {}",
        panel_state(),
        on(|c| win32::is_visible(c.panel.hwnd)),
        on(|c| win32::is_visible(c.glow.hwnd))
    );

    // Tray menu (the clicks it would get) and a real global hotkey
    let tray = on(|c| {
        let before = (c.settings.bool("popups"), c.settings.bool("pinned"));
        c.on_menu("popups");
        c.on_menu("pin");
        let changed = (c.settings.bool("popups"), c.settings.bool("pinned"));
        c.on_menu("popups");
        c.on_menu("pin");
        c.on_menu("dnd:30");
        let dnd = c.dnd_active();
        c.on_menu("dnd:0");
        (crate::tray::exists(), before, changed, dnd, c.dnd_active(), (c.settings.bool("popups"), c.settings.bool("pinned")) == before)
    });
    log!(
        "tray: icon {} | switches {:?} -> {:?} | dnd on {} then {} | back as before {} (expect true, flipped, true, false, true)",
        tray.0,
        tray.1,
        tray.2,
        tray.3,
        tray.4,
        tray.5
    );
    let hotkey = on(|c| {
        let ok = c.set_pref("hotkey", json!("Control+Alt+Z"));
        let registered = c.hotkey_ok;
        c.set_pref("hotkey", json!(""));
        (ok, registered, c.hotkey_ok)
    });
    log!(
        "hotkey Ctrl+Alt+Z: accepted {} | registered {} | after turning it off {} (expect true, true, false)",
        hotkey.0,
        hotkey.1,
        hotkey.2
    );

    fixes_test();

    // Settings screen, and values it must refuse
    on(|c| c.open_settings("", "selftest"));
    wait(700);
    let (mode, state, view_hidden) = on(|c| {
        let a = c.active();
        (c.settings_mode, c.panel_state, !c.chats.is_visible(&a))
    });
    log!(
        "settings screen: open {mode} | panel {state:?} | chat view hidden {view_hidden} | page shows it {} (expect true, Open, true, true)",
        page_js("panel", "!document.getElementById('settings').hidden")
    );
    shot("10-settings");
    let refused = on(|c| {
        [
            c.set_pref("side", json!("up")),
            c.set_pref("opacity", json!(5)),
            c.set_pref("nope", json!(1)),
            c.set_pref("__proto__", json!({})),
            c.set_pref("popupMax", json!("3")),
            c.set_pref("hotkey", json!("Alt+F4")),
        ]
    });
    log!("bad values refused: {refused:?} (expect all false)");
    on(|c| c.emit("panel", "settings:goto", json!(["popups"])));
    wait(900);
    log!(
        "jump to the pop-up page: {} (expect [true, false, the page title])",
        page_js(
            "panel",
            "JSON.stringify([!document.querySelector('.set-page[data-page=popups]').hidden, !document.querySelector('.set-page[data-page=home]').hidden, document.querySelector('.set-title').textContent])"
        )
    );
    shot("11-settings-popups");
    for section in ["look", "security", "updates"] {
        on(move |c| c.emit("panel", "settings:goto", json!([section])));
        wait(900);
        shot(&format!("12-settings-{section}"));
    }

    // Dock on the left edge: panel, views, tab and glow all move over
    on(|c| c.set_pref("side", json!("left")));
    wait(700);
    let (wa, lb, view_x) = on(|c| {
        let a = c.active();
        (c.target_display().work, win32::window_rect(c.panel.hwnd), c.chats.bounds(&a).map(|r| r.x))
    });
    log!(
        "left side: panel x {} (expect {}) | view x {view_x:?} (expect Some(0)) | body.left {}",
        lb.x,
        wa.x,
        page_js("panel", "document.body.classList.contains('left')")
    );
    shot("13-settings-left");
    on(|c| c.close_settings());
    wait(500);
    shot("14-left-chat");
    close_panel();
    log!("left side: closed panel parked at x {} (expect {})", on(|c| win32::window_rect(c.panel.hwnd).x), wa.x - lb.w);
    let db = on(|c| c.target_display().bounds);
    on(move |c| c.show_tab(Some(db.y + db.h / 2)));
    wait(400);
    log!(
        "left side: tab x {} (expect {}) | tab mirrored {}",
        on(|c| c.tab_bounds().x),
        db.x,
        page_js("tab", "document.body.classList.contains('left')")
    );
    shot("15-left-tab");
    on(|c| c.hide_tab(true));
    wait(300);
    on(move |c| c.set_count(first, 2));
    wait(400);
    log!(
        "left side: glow x {} (expect {}) | visible {}",
        on(|c| win32::window_rect(c.glow.hwnd).x),
        db.x,
        on(|c| win32::is_visible(c.glow.hwnd))
    );
    shot("16-left-glow");
    on(move |c| c.set_count(first, 0));

    // Pop-up settings: corner, do-not-disturb, per-app switch, duration
    on(|c| {
        c.set_pref("popupPosition", json!("bottom-left"));
        c.test_popup();
    });
    wait(900);
    let tb = on(|c| c.toasts_bounds());
    log!("pop-up bottom-left: {tb:?} (expect x near {} and bottom near {})", wa.x - 4, wa.y + wa.h + 4);
    shot("17-popup-bottom-left");
    on(|c| c.toasts_dismiss_all());
    let notify_from = move || view_js(pop_app, "(() => { new Notification('ทดสอบ', { body: 'ข้อความทดสอบ' }); return true; })()");
    on(|c| c.set_dnd(30));
    notify_from();
    wait(900);
    log!("do not disturb: pop-up visible {} (expect false)", on(|c| c.toasts_visible()));
    on(move |c| {
        c.set_dnd(0);
        c.set_app_pref(pop_app, "popups", false);
    });
    notify_from();
    wait(900);
    log!("{pop_app} pop-ups off: pop-up visible {} (expect false)", on(|c| c.toasts_visible()));
    on(move |c| {
        c.set_app_pref(pop_app, "popups", true);
        c.set_pref("popupDuration", json!(5));
    });
    notify_from();
    wait(900);
    log!("pop-ups back on: visible {} (expect true)", on(|c| c.toasts_visible()));
    wait(5600);
    log!("5 s duration: still visible {} (expect false)", on(|c| c.toasts_visible()));
    on(|c| c.set_pref("popupDuration", json!(0)));
    notify_from();
    wait(6500);
    log!("\"until closed\": still visible after 6.5 s {} (expect true)", on(|c| c.toasts_visible()));
    on(|c| {
        c.toasts_dismiss_all();
        c.set_pref("popupDuration", json!(8));
        c.set_pref("popupPosition", json!("top-right"));
    });
    wait(600);

    // Update pop-up -> click -> settings opens on the update section
    on(|c| c.announce_update("9.9.9", true));
    wait(900);
    shot("18-update-popup");
    let clicked = js_click("toasts", ".card");
    wait(1200);
    let (state, mode) = on(|c| (c.panel_state, c.settings_mode));
    log!("after clicking the update pop-up (clicked {clicked}): panel {state:?} | settings {mode} (expect Open, true)");
    on(|c| {
        c.update_announced.clear();
        c.set_pref("side", json!("right"));
    });
    wait(500);
    close_panel();
    log!(
        "back on the right: panel parked at x {} | side {}",
        on(|c| win32::window_rect(c.panel.hwnd).x),
        on(|c| c.settings.str("side").to_string())
    );

    // Languages: the settings screen, the tray and pop-ups follow the chosen language
    let lang_before = on(|c| c.settings.get("lang").clone());
    on(|c| c.open_settings("", "selftest"));
    wait(700);
    for lang in ["en", "th", "zh", "ja", "de"] {
        on(move |c| c.set_pref("lang", json!(lang)));
        wait(500);
        let seen = page_js(
            "panel",
            "JSON.stringify([document.querySelector('#settings h1').textContent, document.documentElement.lang, \
             document.querySelector('[data-goto=\"popups\"]').textContent, document.querySelector('[data-text=\"memory\"]').textContent])",
        );
        log!("language {lang}: {seen} | tray says {:?}", i18n::t(lang, "tray.open", &[]));
        shot(&format!("20-settings-{lang}"));
    }
    on(|c| c.emit("panel", "settings:goto", json!(["apps"])));
    wait(900);
    shot("21-settings-apps-de");
    on(move |c| {
        c.set_pref("lang", lang_before);
        c.close_settings();
    });
    wait(300);
    close_panel();

    // Per-app switches: message text off for one app, unread count off for another
    on(|c| c.toasts_dismiss_all());
    wait(600);
    on(move |c| c.set_app_pref(pop_app, "preview", false));
    notify_from();
    wait(900);
    let body = page_js("toasts", "document.querySelector('.card .body')?.textContent || ''");
    log!("{pop_app} message text off: pop-up body {body} (expect {:?})", on(|c| c.t("toast.sentYou")));
    on(move |c| {
        c.toasts_dismiss_all();
        c.set_app_pref(pop_app, "preview", true);
        c.set_app_pref(first, "badge", false);
        c.set_count(first, 3);
    });
    wait(300);
    let (counted, shown, glow) =
        on(move |c| (c.counts.get(first).copied().unwrap_or(0), c.ui_state()["counts"][first].clone(), win32::is_visible(c.glow.hwnd)));
    log!("{first} unread count off: counted {counted} | shown {shown} | glow {glow} (expect 3, 0, false)");
    on(move |c| {
        c.set_count(first, 0);
        c.set_app_pref(first, "badge", true);
    });

    // RAM saver: an app set to sleep unloads after being unused, and wakes up when opened
    let sleeper = on(|c| {
        let a = c.active();
        c.enabled_apps().into_iter().find(|id| *id != a).unwrap_or(c.enabled_apps()[0])
    });
    let before = on(|c| c.memory_stats());
    on(move |c| {
        c.set_app_pref(sleeper, "sleep", true);
        c.last_used.insert(sleeper.to_string(), 0);
        c.sleep_check();
    });
    wait(2500);
    let after = on(|c| c.memory_stats());
    let (asleep, loaded) = on(move |c| (c.asleep.get(sleeper).copied().unwrap_or(false), c.chats.has(sleeper)));
    log!(
        "{sleeper} sleep: asleep {asleep} | page loaded {loaded} | RAM {} MB -> {} MB ({} -> {} processes) (expect true, false, less)",
        before.total,
        after.total,
        before.processes,
        after.processes
    );
    on(move |c| c.open_panel(Some(sleeper), "selftest"));
    wait(1500);
    let (asleep, loaded) = on(move |c| (c.asleep.get(sleeper).copied().unwrap_or(false), c.chats.has(sleeper)));
    log!("{sleeper} opened: asleep {asleep} | page loaded {loaded} (expect false, true)");
    on(move |c| c.set_app_pref(sleeper, "sleep", false));
    close_panel();
    let m = on(|c| c.memory_stats());
    log!("RAM now: {} MB in {} processes", m.total, m.processes);

    edge_test(true);

    // Updating: the "Updating ChatDock" window, then the start after an update
    log!(
        "build names: {} | {} | {} (expect Beta Build 1.7.4, Beta Build 1.6, Beta Build 2.0.1)",
        i18n::build_name("en", &rt::version()),
        i18n::build_name("en", "1.6.0"),
        i18n::build_name("en", "2.0.1")
    );
    on(|c| c.show_update_window("1.6.0"));
    wait(1300);
    shot("30-update-window");
    log!(
        "update window: {} | visible {}",
        page_js("update", "JSON.stringify([document.querySelector('.top b').textContent, document.getElementById('route').textContent, document.getElementById('step').textContent])"),
        on(|c| c.update_win.as_ref().is_some_and(|w| win32::is_visible(w.hwnd)))
    );
    wait(1000);
    on(|c| {
        if let Some(w) = c.update_win.take() {
            let _ = w.w.destroy();
        }
    });
    on(|c| {
        c.settings.set("whatsNew", json!({ "version": rt::version(), "from": "1.4.0", "notes": "", "at": rt::epoch_ms() }));
        c.announce_updated();
    });
    wait(2500);
    shot("31-whats-new-window");
    log!(
        "what's new window: visible {} | {}",
        on(|c| c.whatsnew.win.as_ref().is_some_and(|w| win32::is_visible(w.hwnd))),
        page_js(
            "whatsnew",
            "JSON.stringify([document.getElementById('title').textContent, document.getElementById('route').textContent, [...document.querySelectorAll('.notes h3')].map(h => h.textContent)])"
        )
    );
    let clicked = js_click("whatsnew", "#ok");
    wait(700);
    log!("what's new closed with Got it (clicked {clicked}): {} (expect true)", on(|c| c.whatsnew.win.is_none()));
    on(|c| c.open_settings("updates", "selftest"));
    wait(1500);
    log!(
        "settings, Updates: notes box {} (expect [true, what's new…])",
        page_js(
            "panel",
            "JSON.stringify([!document.querySelector('.uc-notes').hidden, document.querySelector('[data-text=notesTitle]').textContent])"
        )
    );
    shot("32-whats-new");
    close_panel();
}

/// The update chain and ChatDock's own pages, against a feed on this PC that misbehaves on purpose:
/// a check or a download that stalls ends in an error in time (instead of a status stuck until a
/// restart), a download far bigger than an installer is stopped, an installer signed as another
/// version's is refused before it downloads, and a feed only counts when it really is on this PC.
/// The installer lands in a new random folder, locked against changes until it runs. No WEBVIEW2_*
/// variable is left (run it with WEBVIEW2_CHATDOCK_SELFTEST=1 set). ChatDock's own pages may listen
/// (and send ui_send) but not emit to other windows; their own CSP (a <meta> in each page) refuses a
/// script from elsewhere.
fn updater_test() {
    // WEBVIEW2_*
    let left: Vec<String> = std::env::vars_os()
        .map(|(n, _)| n.to_string_lossy().to_string())
        .filter(|n| n.to_ascii_uppercase().starts_with("WEBVIEW2_"))
        .collect();

    // ChatDock's own pages: what their IPC and their CSP allow
    let ask = |expr: &str| -> String {
        page_js(
            "panel",
            &format!("window.__cdAsk = 'pending'; Promise.resolve().then(() => {expr}).then(() => {{ window.__cdAsk = 'allowed'; }}, (e) => {{ window.__cdAsk = 'refused: ' + String(e).slice(0, 60); }}); true"),
        );
        for _ in 0..30 {
            wait(100);
            let r = page_js("panel", "window.__cdAsk");
            if r != "\"pending\"" {
                return r;
            }
        }
        "timeout".into()
    };
    let listen = ask("window.__TAURI__.event.listen('selftest-listen', () => {})");
    let emit = ask("window.__TAURI__.event.emit('selftest-emit', 1)");
    // a script from elsewhere put into the page: what the browser says about it (its CSP refuses it
    // before any request; .invalid never resolves anyway)
    page_js(
        "panel",
        "window.__cdCsp = 'none'; document.addEventListener('securitypolicyviolation', (e) => { window.__cdCsp = 'refused by ' + e.violatedDirective; }, { once: true });          const s = document.createElement('script'); s.src = 'https://selftest.invalid/x.js'; document.head.appendChild(s); true",
    );
    wait(800);
    let inline = page_js("panel", "window.__cdCsp");
    log!(
        "updater: WEBVIEW2_ variables left {left:?} | a ChatDock page: listen {listen}, emit to other windows {emit}, a script from elsewhere put in it {inline} (expect [], \"allowed\", \"refused: ...\", \"refused by script-src-elem\" (the page's own CSP, from before))"
    );

    // the installer on disk
    let random = |name: &str| name.starts_with("ChatDock-update-") && name.len() == "ChatDock-update-".len() + 16;
    let staged = match crate::updater::stage_installer("0.0.0", b"MZ selftest") {
        Ok((file, lock)) => {
            let dir = file.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let unchangeable = std::fs::OpenOptions::new().write(true).open(&file).is_err();
            let undeletable = std::fs::remove_file(&file).is_err();
            let again = crate::updater::stage_installer("0.0.0", b"MZ selftest").ok();
            let own_folder = again.as_ref().is_some_and(|(f2, _)| f2.parent() != Some(dir.as_path()));
            drop(lock);
            if let Some((f2, l2)) = again {
                drop(l2);
                if let Some(d2) = f2.parent() {
                    let _ = std::fs::remove_dir_all(d2);
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
            format!("folder random {}, another gets its own {own_folder} | while it waits: can't be changed {unchangeable}, deleted {undeletable}", random(&name))
        }
        Err(e) => format!("err {e}"),
    };
    log!("updater: the installer: {staged} (expect folder random true, another gets its own true | can't be changed true, deleted true)");

    // the feed
    let (port, sent_big, old_asked) = bad_feed();
    std::env::set_var("CHATDOCK_UPDATE_FEED", "http://127.0.0.1:1@example.com/latest.json");
    let tricked = crate::updater::test_feed();
    let run = |path: &str| -> (String, String, u128) {
        std::env::set_var("CHATDOCK_UPDATE_FEED", format!("http://127.0.0.1:{port}{path}"));
        on(|c| {
            c.upd.enabled = true;
            c.upd.status = "idle";
            c.settings.set("updateAutoDownload", json!(true));
            c.check_update();
        });
        let start = std::time::Instant::now();
        while start.elapsed().as_millis() < 12_000 {
            wait(200);
            if ["error", "ready", "latest", "available"].contains(&on(|c| c.upd.status)) {
                break;
            }
        }
        let (status, error) = on(|c| (c.upd.status.to_string(), crate::core::clean_text(&c.upd.error, 70)));
        (status, error, start.elapsed().as_millis())
    };
    let stall = run("/stall.json");
    let old = run("/old.json");
    let big = run("/big.json");
    let slow = run("/slow.json");
    let watchdog = on(|c| {
        c.upd.status = "checking";
        let attempt = c.next_update_attempt();
        c.update_stuck(attempt, "checking");
        c.upd.status
    });
    std::env::remove_var("CHATDOCK_UPDATE_FEED");
    on(|c| {
        c.upd.enabled = false;
        c.upd.status = "dev";
    });
    let mb = sent_big.load(Ordering::Relaxed) / (1024 * 1024);
    log!(
        "updater: a feed that only starts like this PC {tricked:?} | one that never answers: {} after {} ms ({}) | an installer signed as another version's: {} ({}), asked for {} times | a download far bigger than an installer: {} ({}), {mb} MB sent | one that stalls: {} after {} ms ({}) | the watchdog: {watchdog} (expect None | error after 3-5 s (net::ERR...) | error (signed as another version's), 0 | error (far bigger), under 32 | error after 4-6 s (net::ERR...) | error)",
        stall.0, stall.2, stall.1, old.0, old.1, old_asked.load(Ordering::Relaxed), big.0, big.1, slow.0, slow.2, slow.1
    );
}

/// A feed on this PC that misbehaves on purpose (updater_test), by path. Counts what it sent of
/// the big download and how often the older installer was asked for.
fn bad_feed() -> (u16, std::sync::Arc<std::sync::atomic::AtomicU64>, std::sync::Arc<std::sync::atomic::AtomicU64>) {
    use std::{
        io::{Read, Write},
        sync::{atomic::AtomicU64, Arc},
    };
    let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else { return (0, Arc::default(), Arc::default()) };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let (sent_big, old_asked) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let (sent, asked) = (sent_big.clone(), old_asked.clone());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let (sent, asked) = (sent.clone(), asked.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let feed = |signed_as: &str, file: &str| {
                    format!(
                        r#"{{"version":"99.0.0","notes":"self-test","pub_date":"2026-01-01T00:00:00Z","platforms":{{"windows-x86_64":{{"signature":"{}","url":"http://127.0.0.1:{port}/{file}"}}}}}}"#,
                        fake_signature(signed_as)
                    )
                };
                let answer = |body: String| {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = match path.as_str() {
                    "/stall.json" => {
                        std::thread::sleep(Duration::from_secs(60)); // (answers nothing)
                        Ok(())
                    }
                    "/old.json" => stream.write_all(answer(feed("1.5.0", "old.exe")).as_bytes()),
                    "/big.json" => stream.write_all(answer(feed("99.0.0", "big.exe")).as_bytes()),
                    "/slow.json" => stream.write_all(answer(feed("99.0.0", "slow.exe")).as_bytes()),
                    "/old.exe" => {
                        asked.fetch_add(1, Ordering::Relaxed);
                        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    }
                    "/big.exe" => {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 1073741824\r\nConnection: close\r\n\r\n");
                        let chunk = vec![b'M'; 64 * 1024];
                        // up to 48 MB (an unlimited download keeps it all in memory)
                        while sent.load(Ordering::Relaxed) < 48 * 1024 * 1024 && stream.write_all(&chunk).is_ok() {
                            sent.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                        }
                        Ok(())
                    }
                    "/slow.exe" => {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 5000000\r\nConnection: close\r\n\r\nMZ");
                        std::thread::sleep(Duration::from_secs(60));
                        Ok(())
                    }
                    _ => stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                };
            });
        }
    });
    (port, sent_big, old_asked)
}

/// A minisign signature's text (not a valid signature) naming the installer it was "made" for.
fn fake_signature(version: &str) -> String {
    let text = format!(
        "untrusted comment: self-test\nRWQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\ntrusted comment: timestamp:0\tfile:ChatDock_{version}_x64-setup.exe\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==\n"
    );
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in text.as_bytes().chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}
