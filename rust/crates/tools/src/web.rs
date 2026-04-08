//! Browser/web search tools — give agents access to the internet.
//!
//! Two tools:
//! - `web_search`: Search the web via `DuckDuckGo` Lite (no API key needed)
//! - `web_fetch`: Fetch and extract text content from a URL
//!
//! Both use `curl` under the hood — no extra Rust dependencies.

use std::process::Command;

use serde::{Deserialize, Serialize};

/// Input for `web_search` tool.
#[derive(Debug, Deserialize)]
pub struct WebSearchInput {
    pub query: String,
    /// Maximum number of results to return (default: 5, max: 10).
    pub max_results: Option<usize>,
}

/// A single search result.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Output of `web_search`.
#[derive(Debug, Serialize)]
pub struct WebSearchOutput {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub result_count: usize,
}

/// Input for `web_fetch` tool.
#[derive(Debug, Deserialize)]
pub struct WebFetchInput {
    pub url: String,
    /// Maximum content length in characters (default: 8000).
    pub max_length: Option<usize>,
}

/// Output of `web_fetch`.
#[derive(Debug, Serialize)]
pub struct WebFetchOutput {
    pub url: String,
    pub content: String,
    pub content_length: usize,
    pub truncated: bool,
}

/// Execute a web search using `DuckDuckGo` Lite HTML scraping.
pub fn web_search(input: &WebSearchInput) -> Result<WebSearchOutput, String> {
    let max = input.max_results.unwrap_or(5).min(10);
    let encoded_query = urlencod(&input.query);

    // Use DuckDuckGo Lite — plain HTML, no JS required, no API key
    let url = format!("https://lite.duckduckgo.com/lite/?q={encoded_query}");

    let html = curl_get(&url, 15)?;
    let results = parse_ddg_lite(&html, max);

    Ok(WebSearchOutput {
        query: input.query.clone(),
        result_count: results.len(),
        results,
    })
}

/// Fetch a URL and extract readable text content.
pub fn web_fetch(input: &WebFetchInput) -> Result<WebFetchOutput, String> {
    let max_len = input.max_length.unwrap_or(8000);

    // Validate URL
    if !input.url.starts_with("http://") && !input.url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    let raw = curl_get(&input.url, 30)?;

    // Strip HTML tags to get readable text
    let text = strip_html(&raw);
    let cleaned = collapse_whitespace(&text);

    let truncated = cleaned.len() > max_len;
    let content = if truncated {
        cleaned[..max_len].to_string()
    } else {
        cleaned
    };

    Ok(WebFetchOutput {
        url: input.url.clone(),
        content_length: content.len(),
        truncated,
        content,
    })
}

/// Simple URL encoding for query strings.
fn urlencod(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

/// Fetch a URL using curl.
fn curl_get(url: &str, timeout_secs: u64) -> Result<String, String> {
    let output = Command::new("curl")
        .args([
            "-sL",                          // silent, follow redirects
            "--max-time", &timeout_secs.to_string(),
            "-H", "User-Agent: TachyAgent/0.1 (https://github.com/Anteneh-T-Tessema/tachyagent)",
            url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl error ({}): {}", output.status, stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse `DuckDuckGo` Lite HTML results.
fn parse_ddg_lite(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // DDG Lite uses <a rel="nofollow" ...> for result links
    // and <td class="result-snippet"> for snippets
    let mut pos = 0;
    while results.len() < max_results {
        // Find next result link
        let link_marker = "rel=\"nofollow\"";
        let link_pos = match html[pos..].find(link_marker) {
            Some(p) => pos + p,
            None => break,
        };

        // Extract href
        let href_start = if let Some(p) = html[..link_pos].rfind("href=\"") { p + 6 } else { pos = link_pos + link_marker.len(); continue; };
        let href_end = if let Some(p) = html[href_start..].find('"') { href_start + p } else { pos = link_pos + link_marker.len(); continue; };
        let url = html[href_start..href_end].to_string();

        // Skip DDG internal links
        if url.starts_with('/') || url.contains("duckduckgo.com") {
            pos = link_pos + link_marker.len();
            continue;
        }

        // Extract link text (title)
        let tag_end = if let Some(p) = html[link_pos..].find('>') { link_pos + p + 1 } else { pos = link_pos + link_marker.len(); continue; };
        let close_a = if let Some(p) = html[tag_end..].find("</a>") { tag_end + p } else { pos = link_pos + link_marker.len(); continue; };
        let title = strip_html(&html[tag_end..close_a]).trim().to_string();

        // Extract snippet — look for the next <td class="result-snippet">
        let snippet_marker = "result-snippet";
        let snippet = if let Some(sp) = html[close_a..].find(snippet_marker) {
            let snippet_start = close_a + sp;
            let td_end = html[snippet_start..].find('>').map_or(snippet_start, |p| snippet_start + p + 1);
            let td_close = html[td_end..].find("</td>").map_or(td_end, |p| td_end + p);
            strip_html(&html[td_end..td_close]).trim().to_string()
        } else {
            String::new()
        };

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult { title, url, snippet });
        }

        pos = close_a;
    }

    results
}

/// Strip HTML tags from a string.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;

    let lower = html.to_lowercase();
    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag && i + 7 < lower_chars.len() {
            let ahead: String = lower_chars[i..i+7].iter().collect();
            if ahead == "<script" { in_script = true; }
            if ahead == "<style " || (i + 6 < lower_chars.len() && lower_chars[i..i+6].iter().collect::<String>() == "<style") {
                in_style = true;
            }
        }
        if in_script {
            if i + 9 <= lower_chars.len() && lower_chars[i..i+9].iter().collect::<String>() == "</script>" {
                in_script = false;
                i += 9;
                continue;
            }
            i += 1;
            continue;
        }
        if in_style {
            if i + 8 <= lower_chars.len() && lower_chars[i..i+8].iter().collect::<String>() == "</style>" {
                in_style = false;
                i += 8;
                continue;
            }
            i += 1;
            continue;
        }
        if chars[i] == '<' {
            in_tag = true;
            // Add space for block elements
            if i + 1 < chars.len() {
                let next = lower_chars[i+1];
                if matches!(next, 'p' | 'd' | 'h' | 'l' | 'b' | 't') {
                    out.push(' ');
                }
            }
        } else if chars[i] == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(chars[i]);
        }
        i += 1;
    }

    // Decode common HTML entities
    out.replace("&amp;", "&")
       .replace("&lt;", "<")
       .replace("&gt;", ">")
       .replace("&quot;", "\"")
       .replace("&#39;", "'")
       .replace("&nbsp;", " ")
}

/// Collapse multiple whitespace characters into single spaces.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encoding() {
        assert_eq!(urlencod("hello world"), "hello+world");
        assert_eq!(urlencod("rust lang"), "rust+lang");
        assert_eq!(urlencod("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn strip_html_removes_tags() {
        let html = "<p>Hello <b>world</b></p>";
        let text = strip_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("<b>"));
    }

    #[test]
    fn strip_html_removes_scripts() {
        let html = "before<script>alert('xss')</script>after";
        let text = strip_html(html);
        assert!(text.contains("before"));
        assert!(text.contains("after"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn collapse_whitespace_works() {
        assert_eq!(collapse_whitespace("  hello   world  "), "hello world");
        assert_eq!(collapse_whitespace("a\n\n\nb"), "a b");
    }

    #[test]
    fn web_fetch_rejects_bad_urls() {
        let input = WebFetchInput { url: "ftp://bad".to_string(), max_length: None };
        let err = web_fetch(&input).unwrap_err();
        assert!(err.contains("http"));
    }

    #[test]
    fn parse_ddg_lite_handles_empty() {
        let results = parse_ddg_lite("", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn parse_ddg_lite_extracts_results() {
        let html = r#"
        <a href="https://example.com" rel="nofollow">Example Site</a>
        <td class="result-snippet">This is a snippet about example.</td>
        <a href="https://other.com" rel="nofollow">Other Site</a>
        <td class="result-snippet">Another snippet.</td>
        "#;
        let results = parse_ddg_lite(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Site");
        assert_eq!(results[0].url, "https://example.com");
        assert!(results[0].snippet.contains("snippet about example"));
        assert_eq!(results[1].title, "Other Site");
    }
}
