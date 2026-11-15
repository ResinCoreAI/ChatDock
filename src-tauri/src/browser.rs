//! The Browser tab: ChatDock's own small web browser (added from "+"). Its pages run in a WebView2
//! browser process of their own (data_dir/WebView2-browser), apart from the chats' logins, with
//! SmartScreen on. Its tab strip, navigation row and start page are drawn by the panel page
//! (ui/browser.js); the web pages are WebView2 views stacked under that bar.

use serde_json::{json, Value};

use crate::{browser_tabs::Tabs, core::Core, log};

pub const ID: &str = "browser";

#[derive(Default)]
pub struct Browser {
    pub tabs: Tabs,
}

impl Browser {
    pub fn restore(&mut self, saved: &Value, now: i64) {
        let open = saved.get("open").and_then(Value::as_u64).unwrap_or(0) as usize;
        self.tabs = Tabs::restore(&saved["tabs"], open, now);
    }
}

/// ChatDock's tabs in order: the chat apps, then the Browser when it's on.
pub fn tab_order(mut apps: Vec<&'static str>, browser_on: bool) -> Vec<&'static str> {
    if browser_on {
        apps.push(ID);
    }
    apps
}

impl Core {
    pub fn browser_on(&self) -> bool {
        self.settings.app_on(ID)
    }

    /// Switched on from "+" / off from its tab's menu. Off closes its pages and forgets its tabs
    /// (shortcuts, engine and recent searches stay).
    pub fn set_browser_enabled(&mut self, on: bool) {
        if self.browser_on() == on {
            return;
        }
        self.settings.set_in("apps", ID, json!(on));
        if !on {
            self.browser_close_all();
            self.browser.tabs = Tabs::default();
            self.browser_save();
            if self.active() == ID {
                let first = self.enabled_apps()[0];
                self.set_active(first, false);
            }
        }
        self.save_soon();
        log!("browser {}", if on { "on" } else { "off" });
        self.layout_views();
        self.broadcast_state();
    }

    /// The Browser's tabs and open tab into settings.json.
    pub fn browser_save(&mut self) {
        let mut b = self.settings.get(ID).clone();
        b["tabs"] = self.browser.tabs.saved();
        b["open"] = json!(self.browser.tabs.open_index());
        self.settings.set(ID, b);
        self.save_soon();
    }

    pub fn browser_close_all(&mut self) {} // Task 6

    pub fn browser_reload(&mut self) {} // Task 6
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_browser_comes_after_the_chats() {
        assert_eq!(super::tab_order(vec!["instagram", "discord"], true), ["instagram", "discord", "browser"]);
        assert_eq!(super::tab_order(vec!["instagram"], false), ["instagram"]);
    }
}
