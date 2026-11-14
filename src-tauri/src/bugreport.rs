//! Settings → About → "Report a bug": opens a new GitHub issue with what helps to find a bug
//! already filled in (the build, Windows, the language, which apps are on) and headings for the
//! user to fill. Nothing from the chats goes in, and the log stays on the PC (the issue says where
//! it is, for the user to attach if they want).

/// The issue page, filled in.
pub fn issue_url(repo: &str, title: &str, body: &str) -> String {
    format!("{repo}/issues/new?labels=bug&title={}&body={}", encode(title), encode(body))
}

/// Percent-encoding for a URL query value (UTF-8; letters, digits and -_.~ stay as they are).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The issue's text: headings to fill, then the facts about this PC's ChatDock.
pub fn body(build: &str, version: &str, windows: &str, lang: &str, apps: &[&str], monitors: usize) -> String {
    format!(
        "### What happened?\n\n\n### What did you do just before?\n1. \n2. \n\n### What did you expect?\n\n\n\
         ---\n- ChatDock: {build} ({version})\n- Windows: {windows}\n- Language: {lang}\n- Apps on: {}\n- Monitors: {monitors}\n\n\
         _The log can help: Settings → About → Log file (attach chatdock.log here if you like; it has no messages in it)._\n",
        apps.join(", ")
    )
}

/// "Windows 11 24H2 (build 26100)" from the version keys (ProductName still says 10 on 11).
pub fn windows_name(build: u32, display: &str) -> String {
    let major = if build >= 22000 { 11 } else { 10 };
    let display = if display.is_empty() { String::new() } else { format!(" {display}") };
    format!("Windows {major}{display} (build {build})")
}

#[cfg(test)]
mod tests {
    use super::{body, encode, issue_url, windows_name};

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(encode("Safe-_.~09"), "Safe-_.~09");
        let url = issue_url("https://github.com/ResinCoreAI/ChatDock", "[Bug] x", "line\n");
        assert_eq!(url, "https://github.com/ResinCoreAI/ChatDock/issues/new?labels=bug&title=%5BBug%5D%20x&body=line%0A");
    }

    #[test]
    fn the_report_names_the_build_and_windows_only() {
        let b = body("Beta Build 1.8.1", "1.8.1", "Windows 11 24H2 (build 26100)", "th", &["instagram", "facebook"], 2);
        assert!(b.contains("ChatDock: Beta Build 1.8.1 (1.8.1)"));
        assert!(b.contains("Apps on: instagram, facebook"));
        assert!(b.contains("Monitors: 2"));
        assert_eq!(windows_name(26100, "24H2"), "Windows 11 24H2 (build 26100)");
        assert_eq!(windows_name(19045, "22H2"), "Windows 10 22H2 (build 19045)");
        assert_eq!(windows_name(19045, ""), "Windows 10 (build 19045)");
    }
}
