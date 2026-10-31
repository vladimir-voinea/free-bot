//! Browser tools: the agent drives its own tab in the shared browser.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::Value;

use super::{Risk, ToolCtx, ToolDef, ToolOutput, schema, str_arg};

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef::new(
            "browser_open",
            "Open a URL in your browser tab and get the page: its interactive elements (numbered) and text. The browser keeps logins shared with your team.",
            schema(&[("url", "string", "URL to open", true)]),
            Risk::Safe,
            "Using the browser",
        ),
        ToolDef::new(
            "browser_snapshot",
            "Read the current page again: numbered interactive elements and text.",
            schema(&[]),
            Risk::Safe,
            "Using the browser",
        ),
        ToolDef::new(
            "browser_click",
            "Click an element by its number from the latest snapshot.",
            schema(&[("ref", "integer", "Element number", true)]),
            Risk::Safe,
            "Using the browser",
        ),
        ToolDef::new(
            "browser_type",
            "Type text into an input by its number, optionally pressing Enter after.",
            schema(&[
                ("ref", "integer", "Element number", true),
                ("text", "string", "Text to type", true),
                ("submit", "boolean", "Press Enter after typing", false),
            ]),
            Risk::Safe,
            "Using the browser",
        ),
        ToolDef::new(
            "browser_press",
            "Press a key: Enter, Tab, Escape, Backspace, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, PageUp, PageDown or Space.",
            schema(&[("key", "string", "Key name", true)]),
            Risk::Safe,
            "Using the browser",
        ),
        ToolDef::new(
            "browser_scroll",
            "Scroll the page up or down.",
            schema(&[("direction", "string", "up or down", true)]),
            Risk::Safe,
            "Using the browser",
        ),
        ToolDef::new("browser_back", "Go back to the previous page.", schema(&[]), Risk::Safe, "Using the browser"),
    ]
}

pub fn execute(ctx: &ToolCtx, name: &str, args: &Value) -> Option<ToolOutput> {
    if !name.starts_with("browser_") {
        return None;
    }
    // While the user controls the browser, wait for them to hand it back.
    let mut waited = false;
    while ctx.fleet.lock().takeover.contains(&ctx.agent.id) {
        if !waited {
            ctx.fleet.set_status(&ctx.agent.id, ctx.thread, "Waiting: you are using the browser");
            waited = true;
        }
        if ctx.cancel.load(Ordering::Relaxed) {
            return Some(ToolOutput::err("Stopped."));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    let b = ctx.fleet.browser();
    let id = &ctx.agent.id;
    let r = |key: &str| args.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
    let res = match name {
        "browser_open" => match str_arg(args, "url") {
            Some(u) => b.open(id, u),
            None => Err("Give a URL.".into()),
        },
        "browser_snapshot" => b.snapshot(id),
        "browser_click" => match r("ref") {
            Some(n) => b.click(id, n),
            None => Err("Give the element number as ref.".into()),
        },
        "browser_type" => match (r("ref"), args.get("text").and_then(Value::as_str)) {
            (Some(n), Some(t)) => b.type_text(id, n, t, args.get("submit").and_then(Value::as_bool).unwrap_or(false)),
            _ => Err("Give ref and text.".into()),
        },
        "browser_press" => b.press_key(id, str_arg(args, "key").unwrap_or("Enter")),
        "browser_scroll" => b.scroll(id, str_arg(args, "direction").unwrap_or("down") != "up"),
        "browser_back" => b.back(id),
        _ => return None,
    };
    if waited {
        ctx.fleet.set_status(&ctx.agent.id, ctx.thread, "Using the browser");
    }
    Some(match res {
        Ok(t) => ToolOutput::ok(crate::agents::prompt::truncate(&t, 16_000)),
        Err(e) => ToolOutput::err(e),
    })
}
