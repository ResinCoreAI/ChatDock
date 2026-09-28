//! Self-test: `ChatDock.exe --selftest --profile=<test dir> [--shots=<dir>] [--keep] [--selftest-only=edge]`
//! It drives the running app from a thread of its own (every step reaches the app state through
//! later()) and logs what it saw, followed by "(expect …)". Run it on a test profile only: it
//! switches settings around. `--selftest-only=edge` never takes focus, so it can run during a game.

use std::{path::PathBuf, sync::mpsc, time::Duration};

use serde_json::json;
use tauri::Manager;
use webview2_com::{CapturePreviewCompletedHandler, ExecuteScriptCompletedHandler, Microsoft::Web::WebView2::Win32::*};
use windows::{
    core::HSTRING,
    Win32::{
        Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
        },
        System::Com::{STGM_CREATE, STGM_WRITE},
        UI::Shell::SHCreateStreamOnFileEx,
    },
};

use crate::{
    core::{later, Core, PanelState},
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
/// message while away, and a site that counts other things too. The panel isn't really opened
/// (nothing takes the keyboard): "on screen" is set directly.
fn counts_test() {
    let app = on(|c| c.enabled_apps()[0]);
    let count = move || on(move |c| c.counts.get(app).copied().unwrap_or(0));
    let (popups_were, active_was) = on(|c| (c.settings.get("popups").clone(), c.active()));
    on(move |c| {
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
    on(move |c| {
        c.toasts_dismiss_all();
        c.on_title(app, "Instagram");
        c.settings.set("popups", popups_were);
        c.set_setting("active", json!(active_was));
    });
    wait(3300);
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
         for (let i = 0; i < 2; i++) { const b = document.createElement('button'); b.setAttribute('role', 'switch'); b.setAttribute('aria-checked', 'false'); \
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
    log!("discord voice key: registered {registered} | sent {sent} | switches now {state} | pop-up says {said} (expect true, 8, [\"true\",\"false\"], the mic is off)");
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
    let during = on(|c| {
        c.set_app_pref("discord", "sleep", true);
        c.last_used.insert("discord".into(), 0);
        c.sleep_check();
        (c.chats.memory_low("discord"), c.asleep.get("discord").copied().unwrap_or(false))
    });
    view_js("discord", "window.__testCall.forEach((p) => p.close()), 1");
    let hung_up = wait_call(false);
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
        "discord call: memory low before {low_before} | call seen {started} {states} | during: memory low, asleep {during:?} | hung up {hung_up}, memory low {low_after} | again {again}, after a reload {reloaded} | then asleep {slept} (expect true, true [connected x2], (false, false), true, true, true, true, true)"
    );
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
        "build names: {} | {} | {} (expect Beta Build 1.7, Beta Build 1.6, Beta Build 2.0.1)",
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
