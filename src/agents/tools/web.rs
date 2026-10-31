//! Web search and page fetching without a browser.

use std::time::Duration;

use serde_json::Value;

use super::{Risk, ToolCtx, ToolDef, ToolOutput, schema, str_arg};
use crate::agents::prompt::truncate;

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef::new(
            "web_search",
            "Search the web and get the top results with titles, links and snippets.",
            schema(&[("query", "string", "Search query", true)]),
            Risk::Safe,
            "Searching the web",
        ),
        ToolDef::new(
            "web_fetch",
            "Download a web page and get its readable text and links. For pages that need clicking or logging in, use the browser tools.",
            schema(&[("url", "string", "Full URL starting with http:// or https://", true)]),
            Risk::Safe,
            "Reading a web page",
        ),
    ]
}

pub fn execute(_ctx: &ToolCtx, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "web_search" => search(args),
        "web_fetch" => fetch(args),
        _ => return None,
    })
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (compatible; FreeBot/0.2; +https://github.com/vladimir-voinea/free-bot)")
        .build()
        .expect("http client")
}

fn fetch(args: &Value) -> ToolOutput {
    let Some(url) = str_arg(args, "url") else { return ToolOutput::err("Give a URL.") };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return ToolOutput::err("The URL must start with http:// or https://.");
    }
    let resp = match http().get(url).send() {
        Ok(r) => r,
        Err(e) => return ToolOutput::err(format!("Could not open {url}: {}", crate::api::root_cause(&e))),
    };
    let status = resp.status();
    let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body = match resp.text() {
        Ok(b) => b,
        Err(e) => return ToolOutput::err(format!("Could not read the page: {e}")),
    };
    if !status.is_success() {
        return ToolOutput::err(format!("HTTP {status} from {url}"));
    }
    let text = if ctype.contains("html") || body.trim_start().starts_with('<') {
        let (title, text, links) = html_to_text(&body, url);
        let mut s = format!("# {title}\n{url}\n\n{}", truncate(&text, 40_000));
        if !links.is_empty() {
            s.push_str("\n\nLinks:\n");
            for (t, h) in links.iter().take(40) {
                s.push_str(&format!("- {t}: {h}\n"));
            }
        }
        s
    } else {
        truncate(&body, 40_000)
    };
    ToolOutput::ok(text)
}

fn search(args: &Value) -> ToolOutput {
    let Some(q) = str_arg(args, "query") else { return ToolOutput::err("Give a query.") };
    let form = format!("q={}", percent_encode(q));
    let resp =
        http().post("https://html.duckduckgo.com/html/").header("Content-Type", "application/x-www-form-urlencoded").body(form).send();
    let body: String = match resp.and_then(|r| r.text()) {
        Ok(b) => b,
        Err(e) => return ToolOutput::err(format!("Search failed: {}", crate::api::root_cause(&e))),
    };
    let results = parse_ddg(&body);
    if results.is_empty() {
        return ToolOutput::err("No results (the search service may be rate limiting). Try again or open a site with the browser.");
    }
    let lines: Vec<String> = results.iter().take(10).map(|(t, u, s)| format!("- {t}\n  {u}\n  {s}")).collect();
    ToolOutput::ok(lines.join("\n"))
}

/// Pull (title, url, snippet) out of DuckDuckGo's HTML results page.
pub fn parse_ddg(html: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for block in html.split("class=\"result__a\"").skip(1) {
        let href = block.split("href=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("");
        let title = block.split_once('>').and_then(|(_, r)| r.split("</a").next()).unwrap_or("");
        let snippet = block
            .split("class=\"result__snippet\"")
            .nth(1)
            .and_then(|r| r.split_once('>'))
            .and_then(|(_, r)| r.split("</a").next())
            .unwrap_or("");
        let url = decode_ddg_link(&decode_entities(href));
        if !url.is_empty() {
            out.push((strip_tags(title), url, strip_tags(snippet)));
        }
    }
    out
}

fn decode_ddg_link(href: &str) -> String {
    if let Some(i) = href.find("uddg=") {
        let enc = href[i + 5..].split('&').next().unwrap_or("");
        return percent_decode(enc);
    }
    if href.starts_with("//") { format!("https:{href}") } else { href.to_string() }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(out.split_whitespace().collect::<Vec<_>>().join(" ").as_str())
}

pub fn decode_entities(s: &str) -> String {
    let mut out = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ");
    // Numeric entities.
    while let Some(start) = out.find("&#") {
        let Some(end) = out[start..].find(';').map(|e| start + e) else { break };
        let num = &out[start + 2..end];
        let ch = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()
        } else {
            num.parse().ok()
        }
        .and_then(char::from_u32);
        match ch {
            Some(c) if end - start < 12 => out.replace_range(start..=end, &c.to_string()),
            _ => break,
        }
    }
    out
}

/// A small HTML-to-text converter: drops scripts, styles and markup, keeps
/// paragraphs and headings, and collects links.
pub fn html_to_text(html: &str, base: &str) -> (String, String, Vec<(String, String)>) {
    let lower = html.to_lowercase();
    let title = lower
        .find("<title")
        .and_then(|s| lower[s..].find('>').map(|e| s + e + 1))
        .and_then(|s| lower[s..].find("</title").map(|e| (s, s + e)))
        .map(|(s, e)| strip_tags(&html[s..e]))
        .unwrap_or_default();
    let mut text = String::new();
    let mut links = Vec::new();
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < html.len() {
        if bytes[i] == b'<' {
            let end = html[i..].find('>').map(|e| i + e).unwrap_or(html.len() - 1);
            let tag = &lower[i + 1..end.min(lower.len())];
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if !tag.starts_with('/') && matches!(name.as_str(), "script" | "style" | "noscript" | "svg" | "head" | "template") {
                let close = format!("</{name}");
                i = lower[end..].find(&close).map(|e| end + e).unwrap_or(html.len());
                i = html[i..].find('>').map(|e| i + e + 1).unwrap_or(html.len());
                continue;
            }
            if matches!(
                name.as_str(),
                "p" | "div"
                    | "br"
                    | "li"
                    | "tr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "section"
                    | "article"
                    | "header"
                    | "footer"
                    | "table"
                    | "ul"
                    | "ol"
                    | "pre"
                    | "blockquote"
            ) {
                text.push('\n');
                if !tag.starts_with('/') && name.starts_with('h') && name.len() == 2 {
                    text.push_str("## ");
                }
                if name == "li" && !tag.starts_with('/') {
                    text.push_str("- ");
                }
            }
            if name == "a" && !tag.starts_with('/') {
                let raw = &html[i + 1..end.min(html.len())];
                if let Some(href) = attr(raw, "href") {
                    let label_end = lower[end..].find("</a").map(|e| end + e).unwrap_or(end);
                    let label = strip_tags(&html[end + 1..label_end.max(end + 1)]);
                    let href = absolutize(base, &decode_entities(&href));
                    if !label.is_empty() && href.starts_with("http") && links.len() < 200 {
                        links.push((label.chars().take(80).collect(), href));
                    }
                }
            }
            i = end + 1;
        } else {
            let next = html[i..].find('<').map(|e| i + e).unwrap_or(html.len());
            text.push_str(&html[i..next]);
            i = next;
        }
    }
    let decoded = decode_entities(&text);
    let mut out = String::new();
    let mut blank = 0;
    for line in decoded.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() || l == "-" || l == "##" {
            blank += 1;
            if blank == 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&l);
            out.push('\n');
        }
    }
    (title, out.trim().to_string(), links)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let pos = lower.find(&format!("{name}="))?;
    let rest = &tag[pos + name.len() + 1..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        rest[1..].split(quote).next().map(str::to_string)
    } else {
        rest.split(|c: char| c.is_whitespace() || c == '>').next().map(str::to_string)
    }
}

fn absolutize(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = base[scheme_end..].find('/').map(|i| scheme_end + i).unwrap_or(base.len());
    if href.starts_with('/') {
        return format!("{}{href}", &base[..host_end]);
    }
    if href.starts_with('#') || href.starts_with("javascript:") || href.starts_with("mailto:") {
        return String::new();
    }
    let dir_end = base.rfind('/').filter(|&i| i >= host_end).unwrap_or(host_end);
    format!("{}/{href}", &base[..dir_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_text_and_links() {
        let html = r#"<html><head><title>Hi &amp; bye</title><style>.x{}</style></head><body><h1>Title</h1><p>Hello <b>world</b> &#169;</p><script>var x=1</script><a href="/about">About us</a></body></html>"#;
        let (title, text, links) = html_to_text(html, "https://example.com/page");
        assert_eq!(title, "Hi & bye");
        assert!(text.contains("## Title"));
        assert!(text.contains("Hello world ©"));
        assert!(!text.contains("var x"));
        assert_eq!(links, vec![("About us".to_string(), "https://example.com/about".to_string())]);
    }

    #[test]
    fn ddg_results() {
        let html = r#"<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&amp;rut=1">The <b>Rust</b> Language</a> ... <a class="result__snippet" href="x">A language empowering <b>everyone</b>.</a>"#;
        let r = parse_ddg(html);
        assert_eq!(r[0].0, "The Rust Language");
        assert_eq!(r[0].1, "https://www.rust-lang.org/");
        assert_eq!(r[0].2, "A language empowering everyone.");
    }
}
