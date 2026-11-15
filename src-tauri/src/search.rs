//! The Browser's search side: its engines, "is this an address or a search?", and the engines'
//! suggestion answers. Nothing here touches WebView2 or the network, so all of it is unit tested.

use std::net::IpAddr;

use tauri::Url;

/// The longest text the search box takes, the most suggestions shown, the longest address kept.
pub const TEXT_MAX: usize = 2048;
pub const SUGGEST_MAX: usize = 5;
pub const URL_MAX: usize = 4096;

pub struct Engine {
    pub id: &'static str,
    pub name: &'static str,
    search: &'static str,
    suggest: &'static str,
}

/// {lang}: the UI language's Wikipedia. {q}: the words, escaped.
pub const ENGINES: &[Engine] = &[
    Engine {
        id: "google",
        name: "Google",
        search: "https://www.google.com/search?q={q}",
        suggest: "https://suggestqueries.google.com/complete/search?client=firefox&ie=utf-8&oe=utf-8&q={q}",
    },
    Engine { id: "bing", name: "Bing", search: "https://www.bing.com/search?q={q}", suggest: "https://api.bing.com/osjson.aspx?query={q}" },
    Engine { id: "ddg", name: "DuckDuckGo", search: "https://duckduckgo.com/?q={q}", suggest: "https://duckduckgo.com/ac/?q={q}" },
    Engine {
        id: "youtube",
        name: "YouTube",
        search: "https://www.youtube.com/results?search_query={q}",
        suggest: "https://suggestqueries.google.com/complete/search?client=firefox&ds=yt&ie=utf-8&oe=utf-8&q={q}",
    },
    Engine {
        id: "brave",
        name: "Brave",
        search: "https://search.brave.com/search?q={q}",
        suggest: "https://search.brave.com/api/suggest?q={q}",
    },
    Engine {
        id: "wiki",
        name: "Wikipedia",
        search: "https://{lang}.wikipedia.org/w/index.php?search={q}",
        suggest: "https://{lang}.wikipedia.org/w/api.php?action=opensearch&format=json&limit=5&search={q}",
    },
];

#[derive(Debug, PartialEq)]
pub enum Input {
    /// a web address, made whole (https:// added)
    Open(String),
    /// words for the current engine
    Search(String),
}

pub fn engine(id: &str) -> &'static Engine {
    ENGINES.iter().find(|e| e.id == id).unwrap_or(&ENGINES[0])
}

pub fn is_engine(id: &str) -> bool {
    ENGINES.iter().any(|e| e.id == id)
}

fn wiki_lang(lang: &str) -> &'static str {
    match lang {
        "th" => "th",
        "zh" => "zh",
        "ja" => "ja",
        "de" => "de",
        _ => "en",
    }
}

pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn fill(template: &str, q: &str, lang: &str) -> String {
    // {lang} first: the escaped words can't hold a brace
    template.replace("{lang}", wiki_lang(lang)).replace("{q}", &encode(q.trim()))
}

pub fn search_url(engine_id: &str, q: &str, lang: &str) -> String {
    fill(engine(engine_id).search, q, lang)
}

pub fn suggest_url(engine_id: &str, q: &str, lang: &str) -> String {
    fill(engine(engine_id).suggest, q, lang)
}

/// Endings that are file names people search for, not top-level domains.
const FILE_ENDINGS: &[&str] = &["js", "ts", "py", "rs", "md", "txt", "exe", "dll", "json", "zip", "png", "jpg"];

pub fn classify(text: &str) -> Option<Input> {
    let t: String = text.trim().chars().take(TEXT_MAX).collect();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(match Url::parse(&t) {
            Ok(u) if u.host_str().is_some_and(|h| !h.is_empty()) => Input::Open(u.to_string()),
            _ => Input::Search(t),
        });
    }
    if t.chars().any(char::is_whitespace) {
        return Some(Input::Search(t));
    }
    let host_part = t.split(['/', '?', '#']).next().unwrap_or("");
    let host = match host_part.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host_part,
    };
    let localhost = host.eq_ignore_ascii_case("localhost");
    // four numbers 0-255: an address written as numbers (often a device without https)
    let ipv4 = host.parse::<std::net::Ipv4Addr>().is_ok() && host.split('.').count() == 4;
    let named = host.rsplit_once('.').is_some_and(|(name, tld)| {
        tld.chars().count() >= 2
            && tld.chars().all(char::is_alphabetic)
            && !FILE_ENDINGS.contains(&tld.to_ascii_lowercase().as_str())
            && !name.is_empty()
            && name.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
    });
    if !localhost && !named && !ipv4 {
        return Some(Input::Search(t));
    }
    let scheme = if localhost || ipv4 { "http" } else { "https" };
    Some(match Url::parse(&format!("{scheme}://{t}")) {
        Ok(u) => Input::Open(u.to_string()),
        Err(_) => Input::Search(t),
    })
}

pub fn web_url(url: &str) -> bool {
    let l = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

pub fn addr_is_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_link_local() || v.is_loopback() || v.is_unspecified() || v.is_broadcast(),
        IpAddr::V6(v) => match v.to_ipv4_mapped() {
            Some(v4) => addr_is_lan(IpAddr::V4(v4)),
            None => {
                let first = v.segments()[0];
                v.is_loopback() || v.is_unspecified() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
            }
        },
    }
}

pub fn blocked(url: &str) -> bool {
    if crate::apps::is_this_pc(url) {
        return true;
    }
    let Ok(u) = Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or("").trim_start_matches('[').trim_end_matches(']');
    host.parse::<IpAddr>().is_ok_and(addr_is_lan)
}

pub fn parse_suggestions(body: &str) -> Vec<String> {
    use serde_json::Value;
    let Ok(Value::Array(top)) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let words: Vec<&str> = match top.get(1) {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => top.iter().filter_map(|o| o.get("phrase").and_then(Value::as_str)).collect(),
    };
    words.into_iter().map(|s| s.trim().chars().take(200).collect::<String>()).filter(|s| !s.is_empty()).take(SUGGEST_MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_has_a_search_and_a_suggest_address() {
        let ids: Vec<_> = ENGINES.iter().map(|e| e.id).collect();
        assert_eq!(ids, ["google", "bing", "ddg", "youtube", "brave", "wiki"]);
        for e in ENGINES {
            assert!(search_url(e.id, "x", "en").starts_with("https://"), "{}", e.id);
            assert!(suggest_url(e.id, "x", "en").starts_with("https://"), "{}", e.id);
        }
        assert_eq!(engine("nope").id, "google");
        assert!(is_engine("ddg") && !is_engine("yahoo"));
    }

    #[test]
    fn search_addresses_per_engine() {
        assert_eq!(search_url("google", "elden ring boss", "th"), "https://www.google.com/search?q=elden%20ring%20boss");
        assert_eq!(search_url("youtube", "a&b", "en"), "https://www.youtube.com/results?search_query=a%26b");
        assert_eq!(search_url("wiki", "Bangkok", "th"), "https://th.wikipedia.org/w/index.php?search=Bangkok");
        assert_eq!(search_url("wiki", "Bangkok", "fr"), "https://en.wikipedia.org/w/index.php?search=Bangkok");
        assert_eq!(suggest_url("bing", " hi ", "en"), "https://api.bing.com/osjson.aspx?query=hi");
    }

    #[test]
    fn thai_words_are_escaped_as_utf8() {
        assert_eq!(encode("ไทย"), "%E0%B9%84%E0%B8%97%E0%B8%A2");
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode("100% / ?#"), "100%25%20%2F%20%3F%23");
    }

    #[test]
    fn addresses_open_and_words_search() {
        let open = |t: &str| match classify(t) {
            Some(Input::Open(u)) => u,
            other => panic!("{t}: {other:?}"),
        };
        let search = |t: &str| match classify(t) {
            Some(Input::Search(s)) => s,
            other => panic!("{t}: {other:?}"),
        };
        assert_eq!(open("youtube.com"), "https://youtube.com/");
        assert_eq!(open("  https://Example.com/a?b=1 "), "https://example.com/a?b=1");
        assert_eq!(open("http://neverssl.com"), "http://neverssl.com/");
        assert_eq!(open("pantip.com/topic/1"), "https://pantip.com/topic/1");
        assert_eq!(open("localhost:3000"), "http://localhost:3000/");
        assert_eq!(open("shopee.co.th"), "https://shopee.co.th/");
        // an address written as numbers (then refused or opened by the navigation rules)
        assert_eq!(open("192.168.1.1"), "http://192.168.1.1/");
        assert_eq!(open("8.8.8.8:8080/x"), "http://8.8.8.8:8080/x");
        assert_eq!(search("1.2.3"), "1.2.3");
        assert_eq!(search("999.1.1.1"), "999.1.1.1");
        assert_eq!(search("elden ring boss"), "elden ring boss");
        assert_eq!(search("youtube.com cats"), "youtube.com cats");
        assert_eq!(search("1.5"), "1.5");
        assert_eq!(search("v1.8.2"), "v1.8.2");
        assert_eq!(search("e.g"), "e.g");
        assert_eq!(search("node.js"), "node.js"); // a file name, not a site
        assert_eq!(search("ไทย"), "ไทย");
        assert_eq!(search("https://"), "https://");
        assert_eq!(classify("   "), None);
        assert_eq!(search(&"a".repeat(3000)).len(), TEXT_MAX);
    }

    #[test]
    fn this_pc_and_the_lan_are_refused() {
        for u in [
            "http://localhost/",
            "http://127.0.0.1:8080/",
            "http://0x7f.1/",
            "http://[::1]/",
            "http://192.168.1.1/",
            "https://10.0.0.5/",
            "http://172.16.3.4/",
            "http://169.254.169.254/latest",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:192.168.0.1]/",
        ] {
            assert!(blocked(u), "{u}");
        }
        for u in ["https://www.google.com/", "http://8.8.8.8/", "https://172.32.0.1/", "https://[2606:4700::1111]/"] {
            assert!(!blocked(u), "{u}");
        }
    }

    #[test]
    fn only_web_addresses_are_web() {
        assert!(web_url("https://a.com/") && web_url("HTTP://a.com"));
        assert!(!web_url("file:///C:/x") && !web_url("javascript:alert(1)") && !web_url(""));
    }

    #[test]
    fn suggestions_in_both_shapes() {
        assert_eq!(parse_suggestions(r#"["elden",["elden ring","elden ring dlc"]]"#), ["elden ring", "elden ring dlc"]);
        assert_eq!(parse_suggestions(r#"[{"phrase":"cats"},{"phrase":"cat videos"}]"#), ["cats", "cat videos"]);
        assert_eq!(parse_suggestions(r#"["q",["a","b","c","d","e","f","g"]]"#).len(), SUGGEST_MAX);
        assert!(parse_suggestions("<html>").is_empty());
        assert!(parse_suggestions(r#"{"error":1}"#).is_empty());
        assert_eq!(parse_suggestions(r#"["q",["  ","x"]]"#), ["x"]);
    }

    #[test]
    fn suggestions_keep_thai_text() {
        assert_eq!(parse_suggestions(r#"["ไทย",["ไทยรัฐ","ไทยพีบีเอส"]]"#), ["ไทยรัฐ", "ไทยพีบีเอส"]);
    }
}
