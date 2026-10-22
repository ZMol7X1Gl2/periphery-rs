//! Prefilled GitHub issues the user reviews and submits: "Request support"
//! for devices no driver handles (with a lookup for an existing request),
//! and "Report a bug" for errors.

use anyhow::{Context, Result};
use periphery_core::unknown::UnknownDevice;

const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");

/// An issue that already asks for a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Existing {
    pub url: String,
    pub number: u64,
    pub open: bool,
}

fn repo_slug() -> &'static str {
    REPO_URL.trim_end_matches('/').trim_start_matches("https://github.com/")
}

/// Percent-encodes everything but unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// New-issue page for `.github/ISSUE_TEMPLATE/device-request.yml`, with the
/// device's details filled in. `platform` says where Periphery runs.
pub fn new_issue_url(d: &UnknownDevice, platform: &str) -> String {
    let title = format!("Support for {} ({})", d.name(), d.id());
    let details = format!("{}Periphery: {} ({platform})\n", d.report(), env!("CARGO_PKG_VERSION"));
    format!(
        "{REPO_URL}/issues/new?template=device-request.yml&title={}&device={}&ids={}&details={}",
        encode(&title),
        encode(&d.name()),
        encode(&d.id()),
        encode(&details),
    )
}

/// Longest error text put in a bug report; GitHub refuses very long URLs.
const MAX_ERROR: usize = 1500;

/// New-issue page for `.github/ISSUE_TEMPLATE/bug-report.yml` about `error`.
/// `device` names the device involved, if any; `details` is what Periphery
/// knows about it.
pub fn bug_report_url(device: Option<&str>, error: &str, details: &str, platform: &str) -> String {
    let error: String = error.chars().take(MAX_ERROR).collect();
    let first_line = error.lines().next().unwrap_or_default();
    let summary: String = first_line.chars().take(80).collect();
    let title = match device {
        Some(d) => format!("{d}: {summary}"),
        None => summary,
    };
    let details = format!("{details}Periphery: {} ({platform})\n", env!("CARGO_PKG_VERSION"));
    format!(
        "{REPO_URL}/issues/new?template=bug-report.yml&title={}&device={}&error={}&details={}",
        encode(&title),
        encode(device.unwrap_or_default()),
        encode(&error),
        encode(&details),
    )
}

/// GitHub search API URL for issues mentioning the device's VID:PID.
pub fn search_url(d: &UnknownDevice) -> String {
    let q = format!("repo:{} is:issue \"{}\"", repo_slug(), d.id());
    format!("https://api.github.com/search/issues?per_page=5&q={}", encode(&q))
}

/// The first issue in a search API reply, preferring open ones.
pub fn parse_search(json: &str) -> Result<Option<Existing>> {
    let v: serde_json::Value = serde_json::from_str(json).context("unexpected reply from GitHub")?;
    let items = v["items"].as_array().context("unexpected reply from GitHub")?;
    let found: Vec<Existing> = items
        .iter()
        .filter_map(|i| {
            Some(Existing {
                url: i["html_url"].as_str()?.to_string(),
                number: i["number"].as_u64()?,
                open: i["state"].as_str() == Some("open"),
            })
        })
        .collect();
    Ok(found.iter().find(|e| e.open).or(found.first()).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> UnknownDevice {
        UnknownDevice {
            vid: 0x1e7d,
            pid: 0x2e2c,
            product: "Kone Aimo".into(),
            manufacturer: "ROCCAT".into(),
            collections: vec![(0x0001, 0x0002), (0xff00, 0x0001)],
            interfaces: vec![0, 2],
        }
    }

    #[test]
    fn issue_url_is_prefilled() {
        let url = new_issue_url(&device(), "desktop, windows");
        assert!(url.starts_with("https://github.com/ZMol7X1Gl2/periphery-rs/issues/new?template=device-request.yml&title=Support%20for%20ROCCAT%20Kone%20Aimo%20%281e7d%3A2e2c%29"));
        assert!(url.contains("&ids=1e7d%3A2e2c&"));
        assert!(url.contains("ff00%3A0001"));
        assert!(!url.contains(' ') && !url.contains('\n'));
    }

    #[test]
    fn bug_report_is_prefilled() {
        let url = bug_report_url(Some("Keychron Q1"), "no reply to command 0x01", "Device: Keychron Q1 (3434:0110)\n", "desktop, macos");
        assert!(url.starts_with("https://github.com/ZMol7X1Gl2/periphery-rs/issues/new?template=bug-report.yml&title=Keychron%20Q1%3A%20no%20reply"));
        assert!(url.contains("&device=Keychron%20Q1&"));
        assert!(url.contains("3434%3A0110"));
        assert!(url.contains("desktop%2C%20macos"));
        assert!(!url.contains(' ') && !url.contains('\n'));

        let long = "x".repeat(5000);
        let url = bug_report_url(None, &long, "", "web");
        assert!(url.contains("&device=&"));
        assert!(url.len() < 3000);
    }

    #[test]
    fn search_query() {
        let url = search_url(&device());
        assert!(url.ends_with("q=repo%3AZMol7X1Gl2%2Fperiphery-rs%20is%3Aissue%20%221e7d%3A2e2c%22"));
    }

    #[test]
    fn search_prefers_open_issues() {
        let json = r#"{"total_count":2,"items":[
            {"html_url":"https://github.com/o/r/issues/3","number":3,"state":"closed"},
            {"html_url":"https://github.com/o/r/issues/7","number":7,"state":"open"}]}"#;
        assert_eq!(parse_search(json).unwrap(), Some(Existing { url: "https://github.com/o/r/issues/7".into(), number: 7, open: true }));
        assert_eq!(parse_search(r#"{"total_count":0,"items":[]}"#).unwrap(), None);
        assert!(parse_search(r#"{"message":"API rate limit exceeded"}"#).is_err());
    }
}
