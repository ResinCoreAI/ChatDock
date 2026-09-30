//! Tray icon: a left click opens / hides the panel, a right click has the everyday switches
//! (everything else lives on the settings screen). The icon switches to its "unread" look while a
//! chat has news, and the tooltip lists who wrote.
//!
//! Windows shows the tray menu itself, straight from the click, so the menu is built ahead of time
//! and rebuilt when what it shows changes, never while it is open.

use std::cell::{Cell, RefCell};

use tauri::{
    image::Image,
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    Wry,
};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use windows::Win32::{
    System::{
        SystemInformation::{GetLocalTime, GetSystemTime},
        Threading::GetCurrentThreadId,
    },
    UI::{
        Input::KeyboardAndMouse::GetDoubleClickTime,
        WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO, GUI_INMENUMODE},
    },
};

use crate::{
    apps,
    core::{later, Core},
    log, rt,
};

#[derive(Default)]
struct Shown {
    menu_key: String,
    tip: String,
    unread: Option<bool>,
    menus: Vec<Menu<Wry>>, // the one in use and the one before it (it may still be on screen)
}

thread_local! {
    static TRAY: RefCell<Option<TrayIcon<Wry>>> = const { RefCell::new(None) };
    static SHOWN: RefCell<Shown> = RefCell::new(Shown::default());
    /// when the tray icon was last clicked (left button up, ms since 1970)
    static LAST_CLICK: Cell<i64> = const { Cell::new(0) };
}

fn icon(unread: bool) -> Option<Image<'static>> {
    let bytes: &'static [u8] =
        if unread { include_bytes!("../../assets/tray-unread.ico") } else { include_bytes!("../../assets/tray.ico") };
    Image::from_bytes(bytes).ok()
}

/// Take the icon out of the tray (before the update installer closes ChatDock, so no dead icon
/// stays behind) or put it back.
pub fn set_shown(on: bool) {
    TRAY.with(|t| {
        if let Some(t) = t.borrow().as_ref() {
            let _ = t.set_visible(on);
        }
    });
}

pub fn exists() -> bool {
    TRAY.with(|t| t.borrow().is_some())
}

/// The tray icon was clicked less than a double-click's time ago: the second press of a double
/// click is coming or came (Windows sends both clicks' button-ups).
pub fn clicked_just_now() -> bool {
    rt::epoch_ms() - LAST_CLICK.with(Cell::get) < unsafe { GetDoubleClickTime() } as i64
}

/// Our own tray menu (or any menu of ours) is open right now.
fn menu_open() -> bool {
    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut info).is_ok() && (info.flags & GUI_INMENUMODE).0 != 0 }
}

pub fn create(c: &mut Core) {
    let built = TrayIconBuilder::with_id("main")
        .icon(icon(false).expect("tray icon"))
        .tooltip("ChatDock")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|_, event| match event {
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => later(|c| c.tray_click()),
            // about to be right-clicked: make sure the menu is up to date before Windows shows it
            TrayIconEvent::Enter { .. } | TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Down, .. } => {
                later(|c| c.update_tray())
            }
            _ => {}
        })
        .on_menu_event(|_, event| {
            let id = event.id().0.clone();
            later(move |c| c.on_menu(&id));
        })
        .build(rt::app());
    match built {
        Ok(tray) => TRAY.with(|t| *t.borrow_mut() = Some(tray)),
        Err(err) => log!("tray icon failed: {err}"),
    }
    c.update_tray();
}

/// "14:30" for a time given in ms since 1970 (local time, for "do not disturb until …").
fn clock_time(epoch_ms: i64) -> String {
    let (local, utc) = unsafe { (GetLocalTime(), GetSystemTime()) };
    let day = |t: &windows::Win32::Foundation::SYSTEMTIME| (t.wYear, t.wMonth, t.wDay);
    let mut offset = (local.wHour as i64 * 60 + local.wMinute as i64) - (utc.wHour as i64 * 60 + utc.wMinute as i64);
    if day(&local) > day(&utc) {
        offset += 24 * 60;
    } else if day(&local) < day(&utc) {
        offset -= 24 * 60;
    }
    let minutes = (epoch_ms / 60_000 + offset).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

impl Core {
    fn tray_click(&mut self) {
        let since = rt::epoch_ms() - self.last_auto_hide_at;
        log!("tray click sinceAutoHide={since} {}", self.snap());
        if clicked_just_now() {
            // the second click of a double click: the first one already opened (or hid) the chat;
            // the press took the focus to the taskbar, so give it back to an opening chat
            if self.panel_state.showing() {
                self.blurred_while_opening = false;
                self.focus_panel();
            }
            return;
        }
        LAST_CLICK.with(|c| c.set(rt::epoch_ms()));
        if since < 600 {
            return; // the click that just hid the panel
        }
        self.toggle_panel("tray");
    }

    fn dnd_label(&self, on: &str, until_key: &str) -> String {
        let until = self.settings.i64("dndUntil");
        if until == -1 {
            self.t(on)
        } else {
            self.tv(until_key, &[("time", clock_time(until))])
        }
    }

    /// Icon, tooltip and menu follow the state (called with every state broadcast).
    pub fn update_tray(&mut self) {
        let has_tray = TRAY.with(|t| t.borrow().is_some());
        if !has_tray {
            return;
        }
        let unread = self.total_unread() > 0;
        let listed: Vec<String> = self
            .enabled_apps()
            .into_iter()
            .filter(|id| self.shown_count(id) > 0)
            .map(|id| format!("{} {}", apps::get(id).unwrap().name, self.shown_count(id)))
            .collect();
        let hotkey = if self.hotkey_ok { self.hotkey_label() } else { String::new() };
        let update_ready = self.upd.status == "ready";
        let mut tip = vec![
            "ChatDock".to_string(),
            if listed.is_empty() { self.t("tip.noNew") } else { self.tv("tip.new", &[("list", listed.join(" · "))]) },
        ];
        if self.dnd_active() {
            tip.push(self.dnd_label("tip.dnd", "tip.dndUntil"));
        }
        if update_ready {
            tip.push(self.tv("tip.update", &[("version", self.build_name(&self.upd.version))]));
        }
        if !hotkey.is_empty() {
            tip.push(self.tv("tip.hotkey", &[("hotkey", hotkey.clone())]));
        }
        let tip: String = tip.join("\n").chars().take(127).collect(); // Windows cuts tooltips at 127 characters

        let menu_key = format!(
            "{}|{}|{}|{}|{:?}|{}|{}|{}|{}",
            self.lang,
            update_ready,
            self.upd.version,
            self.panel_state == crate::core::PanelState::Hidden,
            self.enabled_apps().iter().map(|id| (id.to_string(), self.shown_count(id))).collect::<Vec<_>>(),
            self.settings.bool("popups"),
            if self.dnd_active() { self.settings.i64("dndUntil") } else { 0 },
            self.settings.bool("pinned"),
            hotkey,
        );

        let (icon_changed, tip_changed, menu_changed) = SHOWN.with(|s| {
            let s = s.borrow();
            (s.unread != Some(unread), s.tip != tip, s.menu_key != menu_key)
        });
        TRAY.with(|t| {
            let t = t.borrow();
            let Some(tray) = t.as_ref() else { return };
            // (remembered only when Windows took it: while Explorer restarts it fails, and is tried again)
            if icon_changed && tray.set_icon(icon(unread)).is_ok() {
                SHOWN.with(|s| s.borrow_mut().unread = Some(unread));
            }
            if tip_changed && tray.set_tooltip(Some(&tip)).is_ok() {
                SHOWN.with(|s| s.borrow_mut().tip = tip.clone());
            }
            if menu_changed && !menu_open() {
                match self.build_menu(update_ready, &hotkey) {
                    Ok(menu) => {
                        let _ = tray.set_menu(Some(menu.clone()));
                        SHOWN.with(|s| {
                            let mut s = s.borrow_mut();
                            s.menu_key = menu_key;
                            s.menus.push(menu);
                            if s.menus.len() > 2 {
                                s.menus.remove(0);
                            }
                        });
                    }
                    Err(err) => log!("tray menu failed: {err}"),
                }
            }
        });
    }

    fn build_menu(&self, update_ready: bool, hotkey: &str) -> tauri::Result<Menu<Wry>> {
        let app = rt::app();
        let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
        let sep = || -> tauri::Result<Box<dyn IsMenuItem<Wry>>> { Ok(Box::new(PredefinedMenuItem::separator(app)?)) };
        let item = |id: &str, text: String| -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
            Ok(Box::new(MenuItem::with_id(app, id, text.replace('&', "&&"), true, None::<&str>)?))
        };
        let check = |id: &str, text: String, on: bool| -> tauri::Result<CheckMenuItem<Wry>> {
            CheckMenuItem::with_id(app, id, text.replace('&', "&&"), true, on, None::<&str>)
        };

        if update_ready {
            items.push(item("update", self.tv("tray.updateNow", &[("version", self.build_name(&self.upd.version))]))?);
            items.push(sep()?);
        }
        // Does what it says when it was built: opening the menu can hide the chat (a click elsewhere)
        // while the menu still shows "Hide"
        let hidden = self.panel_state == crate::core::PanelState::Hidden;
        let (toggle_id, toggle) = if hidden { ("open", self.t("tray.open")) } else { ("hide", self.t("tray.hide")) };
        // the hotkey is shown the way Windows shows shortcuts in menus: right-aligned after a tab
        items.push(item(toggle_id, if hotkey.is_empty() { toggle } else { format!("{toggle}\t{hotkey}") })?);
        for id in self.enabled_apps() {
            let name = apps::get(id).unwrap().name;
            let n = self.shown_count(id);
            items.push(item(&format!("app:{id}"), if n > 0 { format!("{name}  ({n})") } else { name.to_string() })?);
        }
        items.push(sep()?);
        items.push(Box::new(check("popups", self.t("tray.popups"), self.settings.bool("popups"))?));

        let dnd = self.dnd_active();
        let until = self.settings.i64("dndUntil");
        let dnd_title = if dnd { self.dnd_label("tray.dndOn", "tray.dndUntil") } else { self.t("tray.dnd") };
        let choices = [
            ("0", "dnd.off", !dnd),
            ("30", "dnd.30", false),
            ("60", "dnd.60", false),
            ("120", "dnd.120", false),
            ("480", "dnd.480", false),
            ("-1", "dnd.forever", until == -1),
        ];
        let dnd_items: Vec<CheckMenuItem<Wry>> =
            choices.iter().map(|(m, key, on)| check(&format!("dnd:{m}"), self.t(key), *on)).collect::<tauri::Result<_>>()?;
        let dnd_refs: Vec<&dyn IsMenuItem<Wry>> = dnd_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
        items.push(Box::new(Submenu::with_id_and_items(app, "dnd", dnd_title.replace('&', "&&"), true, &dnd_refs)?));
        items.push(Box::new(check("pin", self.t("tray.pin"), self.settings.bool("pinned"))?));
        items.push(sep()?);
        items.push(item("settings", self.t("tray.settings"))?);
        items.push(item("help", self.t("tray.help"))?);
        items.push(item("quit", self.t("tray.quit"))?);

        let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
        Menu::with_items(app, &refs)
    }

    pub fn on_menu(&mut self, id: &str) {
        log!("tray menu {id}");
        SHOWN.with(|s| s.borrow_mut().menu_key.clear()); // Windows ticked / unticked the item itself: rebuild
        match id {
            "update" => self.install_update(),
            "open" => {
                if !self.panel_state.showing() {
                    self.toggle_panel("menu");
                }
            }
            "hide" => {
                if self.panel_state.showing() {
                    self.toggle_panel("menu");
                }
            }
            "popups" => {
                let on = !self.settings.bool("popups");
                self.set_pref("popups", serde_json::json!(on));
            }
            "pin" => {
                let on = !self.settings.bool("pinned");
                self.set_pref("pinned", serde_json::json!(on));
            }
            "settings" => self.open_settings("", "menu"),
            "help" => self.show_help(),
            "quit" => self.quit(),
            _ => {
                if let Some(app) = id.strip_prefix("app:").filter(|a| self.is_enabled(a)) {
                    let app = app.to_string();
                    if self.settings_mode {
                        self.close_settings();
                    }
                    self.open_panel(Some(&app), "menu");
                } else if let Some(m) = id.strip_prefix("dnd:").and_then(|m| m.parse::<i64>().ok()) {
                    self.set_dnd(m);
                }
            }
        }
        self.update_tray();
    }
}

/// The global hotkeys: (what it does, accelerator); "" = none. Gives back, for each one registered,
/// its shortcut id and what it does; one another app holds is left out (and logged).
pub fn register_hotkeys(keys: &[(&'static str, String)]) -> Vec<(u32, &'static str)> {
    use tauri_plugin_global_shortcut::Shortcut;
    let gs = rt::app().global_shortcut();
    let _ = gs.unregister_all();
    let mut out = Vec::new();
    for (action, acc) in keys {
        if acc.is_empty() {
            continue;
        }
        let Ok(shortcut) = acc.parse::<Shortcut>() else {
            log!("hotkey {acc}: not a shortcut");
            continue;
        };
        match gs.register(shortcut) {
            Ok(()) => out.push((shortcut.id(), *action)),
            Err(err) => log!("hotkey {acc}: {err}"),
        }
    }
    out
}
