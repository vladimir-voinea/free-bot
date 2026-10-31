//! The agents' browser. One Chrome/Chromium/Edge process runs headless with a
//! shared profile (so logins are shared across the team), and each agent gets
//! its own tab, driven over the Chrome DevTools Protocol. The UI shows a live
//! view of an agent's tab and lets the user take over, and can record what the
//! user does to teach the agent a skill.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::api::Waker;

pub const VIEW_W: u32 = 1280;
pub const VIEW_H: u32 = 800;

pub struct Frame {
    pub jpeg: Vec<u8>,
    pub url: String,
    pub title: String,
    /// Increments with every new frame so the UI knows when to re-upload the texture.
    pub seq: u64,
}

struct Tab {
    ws: Mutex<WebSocket<TcpStream>>,
    next: AtomicU64,
}

impl Tab {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut ws = self.ws.lock().unwrap_or_else(|e| e.into_inner());
        ws.send(Message::Text(json!({"id": id, "method": method, "params": params}).to_string().into()))
            .map_err(|e| format!("Lost the browser: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if Instant::now() > deadline {
                return Err(format!("The browser did not answer {method}."));
            }
            let msg = ws.read().map_err(|e| format!("Lost the browser: {e}"))?;
            let Message::Text(t) = msg else { continue };
            let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(e) = v.get("error") {
                    return Err(e.get("message").and_then(Value::as_str).unwrap_or("browser error").to_string());
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    fn eval(&self, js: &str) -> Result<Value, String> {
        let r = self.call("Runtime.evaluate", json!({"expression": js, "returnByValue": true, "awaitPromise": true}))?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(ex.pointer("/exception/description").and_then(Value::as_str).unwrap_or("script error").to_string());
        }
        Ok(r.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }
}

struct Proc {
    child: Child,
    port: u16,
}

pub struct BrowserManager {
    profile: PathBuf,
    waker: Arc<dyn Waker>,
    path: Mutex<String>,
    proc_: Mutex<Option<Proc>>,
    tabs: Mutex<HashMap<String, Arc<Tab>>>,
    frames: Arc<Mutex<HashMap<String, Frame>>>,
    seq: AtomicU64,
    /// Agents whose live view is on screen, with the last time it was drawn.
    viewers: Arc<Mutex<HashMap<String, Instant>>>,
    /// Steps recorded while the user teaches a task, per agent.
    recordings: Mutex<HashMap<String, Vec<String>>>,
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        if let Ok(mut p) = self.proc_.lock()
            && let Some(mut p) = p.take()
        {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }
}

impl BrowserManager {
    pub fn new(profile: PathBuf, waker: Arc<dyn Waker>) -> Self {
        Self {
            profile,
            waker,
            path: Mutex::new(String::new()),
            proc_: Mutex::new(None),
            tabs: Mutex::new(HashMap::new()),
            frames: Arc::new(Mutex::new(HashMap::new())),
            seq: AtomicU64::new(1),
            viewers: Arc::new(Mutex::new(HashMap::new())),
            recordings: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_path(&self, p: &str) {
        *self.path.lock().unwrap_or_else(|e| e.into_inner()) = p.to_string();
    }

    pub fn has_tab(&self, agent: &str) -> bool {
        self.tabs.lock().unwrap_or_else(|e| e.into_inner()).contains_key(agent)
    }

    fn http() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().expect("http client")
    }

    fn ensure_proc(&self) -> Result<u16, String> {
        let mut guard = self.proc_.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.as_mut() {
            if matches!(p.child.try_wait(), Ok(None)) {
                return Ok(p.port);
            }
            self.tabs.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        let configured = self.path.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let exe = find_browser(&configured)
            .ok_or("No Chrome, Chromium or Edge browser was found. Install one, or set its path in Settings → Agents → Browser.")?;
        let _ = std::fs::create_dir_all(&self.profile);
        let mut cmd = Command::new(&exe);
        cmd.arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", self.profile.display()))
            .arg(format!("--window-size={VIEW_W},{VIEW_H}"))
            .args(["--no-first-run", "--no-default-browser-check", "--disable-gpu", "--mute-audio", "--hide-scrollbars"]);
        if running_as_root() {
            cmd.arg("--no-sandbox");
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("Could not start the browser ({}): {e}", exe.display()))?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(rest) = line.split("ws://127.0.0.1:").nth(1)
                    && let Some(port) = rest.split('/').next().and_then(|p| p.parse::<u16>().ok())
                {
                    let _ = tx.send(port);
                }
            }
        });
        let port = rx.recv_timeout(Duration::from_secs(25)).map_err(|_| {
            let _ = child.kill();
            "The browser started but did not open its control port.".to_string()
        })?;
        *guard = Some(Proc { child, port });
        Ok(port)
    }

    fn tab(&self, agent: &str) -> Result<Arc<Tab>, String> {
        if let Some(t) = self.tabs.lock().unwrap_or_else(|e| e.into_inner()).get(agent) {
            return Ok(t.clone());
        }
        let port = self.ensure_proc()?;
        let resp = Self::http()
            .put(format!("http://127.0.0.1:{port}/json/new?about:blank"))
            .send()
            .and_then(|r| r.json::<Value>())
            .map_err(|e| format!("Could not open a browser tab: {e}"))?;
        let ws_url = resp.get("webSocketDebuggerUrl").and_then(Value::as_str).ok_or("The browser did not give a tab address.")?;
        let hostport = ws_url.trim_start_matches("ws://").split('/').next().unwrap_or("").to_string();
        let stream = TcpStream::connect(&hostport).map_err(|e| format!("Could not connect to the browser: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(45))).ok();
        let (ws, _) = tungstenite::client(ws_url, stream).map_err(|e| format!("Could not connect to the browser: {e}"))?;
        let tab = Arc::new(Tab { ws: Mutex::new(ws), next: AtomicU64::new(1) });
        tab.call("Page.enable", json!({}))?;
        tab.call(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": VIEW_W, "height": VIEW_H, "deviceScaleFactor": 1, "mobile": false}),
        )?;
        self.tabs.lock().unwrap_or_else(|e| e.into_inner()).insert(agent.to_string(), tab.clone());
        self.start_refresher(agent.to_string(), tab.clone());
        Ok(tab)
    }

    /// Keeps a fresh screenshot while someone is watching the live view.
    fn start_refresher(&self, agent: String, tab: Arc<Tab>) {
        let frames = self.frames.clone();
        let viewers = self.viewers.clone();
        let waker = self.waker.clone();
        let seq = Arc::new(AtomicU64::new(self.seq.fetch_add(1000, Ordering::Relaxed)));
        std::thread::spawn(move || {
            loop {
                if Arc::strong_count(&tab) == 1 {
                    return; // the tab was closed
                }
                let watching =
                    viewers.lock().unwrap_or_else(|e| e.into_inner()).get(&agent).is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                if watching {
                    if let Ok(f) = capture(&tab) {
                        let mut map = frames.lock().unwrap_or_else(|e| e.into_inner());
                        map.insert(agent.clone(), Frame { seq: seq.fetch_add(1, Ordering::Relaxed), ..f });
                        drop(map);
                        waker.wake();
                    }
                    std::thread::sleep(Duration::from_millis(700));
                } else {
                    std::thread::sleep(Duration::from_millis(400));
                }
            }
        });
    }

    pub fn close_tab(&self, agent: &str) {
        self.tabs.lock().unwrap_or_else(|e| e.into_inner()).remove(agent);
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).remove(agent);
    }

    /// Mark the live view of `agent` as visible. Starts the browser tab if needed (in the background).
    pub fn watch(&self, agent: &str) {
        self.viewers.lock().unwrap_or_else(|e| e.into_inner()).insert(agent.to_string(), Instant::now());
    }

    pub fn with_frame<R>(&self, agent: &str, f: impl FnOnce(Option<&Frame>) -> R) -> R {
        let map = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        f(map.get(agent))
    }

    fn refresh_now(&self, agent: &str, tab: &Tab) {
        if let Ok(f) = capture(tab) {
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).insert(agent.to_string(), Frame { seq, ..f });
            self.waker.wake();
        }
    }

    // ---- actions used by agent tools ----------------------------------------

    pub fn open(&self, agent: &str, url: &str) -> Result<String, String> {
        let tab = self.tab(agent)?;
        let url = if url.contains("://") || url.starts_with("about:") { url.to_string() } else { format!("https://{url}") };
        let r = tab.call("Page.navigate", json!({"url": url}))?;
        if let Some(e) = r.get("errorText").and_then(Value::as_str) {
            return Err(format!("Could not open {url}: {e}"));
        }
        wait_ready(&tab);
        self.refresh_now(agent, &tab);
        self.snapshot_with(&tab)
    }

    pub fn snapshot(&self, agent: &str) -> Result<String, String> {
        let tab = self.tab(agent)?;
        self.snapshot_with(&tab)
    }

    fn snapshot_with(&self, tab: &Tab) -> Result<String, String> {
        let v = tab.eval(SNAPSHOT_JS)?;
        let s = v.as_str().unwrap_or("{}");
        let v: Value = serde_json::from_str(s).unwrap_or_default();
        let mut out = format!(
            "Page: {}\nURL: {}\n\nInteractive elements (use the number as ref):\n",
            v["title"].as_str().unwrap_or(""),
            v["url"].as_str().unwrap_or("")
        );
        for e in v["elements"].as_array().cloned().unwrap_or_default() {
            out.push_str(e.as_str().unwrap_or(""));
            out.push('\n');
        }
        out.push_str("\nPage text:\n");
        out.push_str(v["text"].as_str().unwrap_or(""));
        Ok(out)
    }

    fn element_center(tab: &Tab, r: u64) -> Result<(f64, f64), String> {
        let v = tab.eval(&format!(
            "(() => {{ const e = document.querySelector('[data-fb-ref=\"{r}\"]'); if (!e) return null; e.scrollIntoView({{block: 'center', inline: 'center'}}); const b = e.getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]; }})()"
        ))?;
        let arr = v.as_array().ok_or(format!("No element {r} on the page. Take a new snapshot first."))?;
        Ok((arr[0].as_f64().unwrap_or(0.0), arr[1].as_f64().unwrap_or(0.0)))
    }

    fn mouse_click(tab: &Tab, x: f64, y: f64) -> Result<(), String> {
        tab.call("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y}))?;
        tab.call("Input.dispatchMouseEvent", json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1}))?;
        tab.call("Input.dispatchMouseEvent", json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1}))?;
        Ok(())
    }

    pub fn click(&self, agent: &str, r: u64) -> Result<String, String> {
        let tab = self.tab(agent)?;
        let (x, y) = Self::element_center(&tab, r)?;
        Self::mouse_click(&tab, x, y)?;
        std::thread::sleep(Duration::from_millis(400));
        wait_ready(&tab);
        self.refresh_now(agent, &tab);
        self.snapshot_with(&tab)
    }

    pub fn type_text(&self, agent: &str, r: u64, text: &str, submit: bool) -> Result<String, String> {
        let tab = self.tab(agent)?;
        let (x, y) = Self::element_center(&tab, r)?;
        Self::mouse_click(&tab, x, y)?;
        tab.eval(&format!(
            "(() => {{ const e = document.querySelector('[data-fb-ref=\"{r}\"]'); if (e && 'value' in e) {{ e.focus(); e.select && e.select(); }} }})()"
        ))?;
        tab.call("Input.insertText", json!({"text": text}))?;
        if submit {
            press(&tab, "Enter")?;
            std::thread::sleep(Duration::from_millis(500));
            wait_ready(&tab);
        }
        self.refresh_now(agent, &tab);
        self.snapshot_with(&tab)
    }

    pub fn press_key(&self, agent: &str, key: &str) -> Result<String, String> {
        let tab = self.tab(agent)?;
        press(&tab, key)?;
        std::thread::sleep(Duration::from_millis(300));
        wait_ready(&tab);
        self.refresh_now(agent, &tab);
        self.snapshot_with(&tab)
    }

    pub fn scroll(&self, agent: &str, down: bool) -> Result<String, String> {
        let tab = self.tab(agent)?;
        tab.eval(&format!("window.scrollBy(0, {})", if down { 600 } else { -600 }))?;
        self.refresh_now(agent, &tab);
        self.snapshot_with(&tab)
    }

    pub fn back(&self, agent: &str) -> Result<String, String> {
        let tab = self.tab(agent)?;
        tab.eval("history.back()")?;
        std::thread::sleep(Duration::from_millis(500));
        wait_ready(&tab);
        self.refresh_now(agent, &tab);
        self.snapshot_with(&tab)
    }

    // ---- the user's own input in the live view (take over, teach a task) ----

    pub fn start_recording(&self, agent: &str) {
        self.recordings.lock().unwrap_or_else(|e| e.into_inner()).insert(agent.to_string(), Vec::new());
    }

    pub fn is_recording(&self, agent: &str) -> bool {
        self.recordings.lock().unwrap_or_else(|e| e.into_inner()).contains_key(agent)
    }

    pub fn stop_recording(&self, agent: &str) -> Option<Vec<String>> {
        self.recordings.lock().unwrap_or_else(|e| e.into_inner()).remove(agent)
    }

    fn record(&self, agent: &str, step: String) {
        if let Some(r) = self.recordings.lock().unwrap_or_else(|e| e.into_inner()).get_mut(agent) {
            r.push(step);
        }
    }

    /// Run a user action off the UI thread.
    fn user_action(self: &Arc<Self>, agent: &str, f: impl FnOnce(&Arc<Self>, &Tab) -> Result<(), String> + Send + 'static) {
        let me = self.clone();
        let agent = agent.to_string();
        std::thread::spawn(move || {
            if let Ok(tab) = me.tab(&agent) {
                if let Err(e) = f(&me, &tab) {
                    eprintln!("browser: {e}");
                }
                me.refresh_now(&agent, &tab);
            }
        });
    }

    pub fn user_click(self: &Arc<Self>, agent: &str, x: f64, y: f64) {
        let a = agent.to_string();
        self.user_action(agent, move |me, tab| {
            if me.is_recording(&a) {
                let what = tab
                    .eval(&format!(
                        "(() => {{ const e = document.elementFromPoint({x}, {y}); if (!e) return ''; const t = (e.getAttribute('aria-label') || e.innerText || e.value || e.getAttribute('placeholder') || e.getAttribute('name') || '').trim().replace(/\\s+/g, ' ').slice(0, 60); return e.tagName.toLowerCase() + (t ? ' \"' + t + '\"' : ''); }})()"
                    ))
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                me.record(&a, format!("Clicked {what}"));
            }
            Self::mouse_click(tab, x, y)?;
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        });
    }

    pub fn user_type(self: &Arc<Self>, agent: &str, text: String) {
        let a = agent.to_string();
        self.user_action(agent, move |me, tab| {
            if me.is_recording(&a) {
                me.record(&a, format!("Typed \"{text}\""));
            }
            tab.call("Input.insertText", json!({"text": text}))?;
            Ok(())
        });
    }

    pub fn user_key(self: &Arc<Self>, agent: &str, key: &'static str) {
        let a = agent.to_string();
        self.user_action(agent, move |me, tab| {
            if me.is_recording(&a) && key != "Backspace" {
                me.record(&a, format!("Pressed {key}"));
            }
            press(tab, key)?;
            std::thread::sleep(Duration::from_millis(200));
            Ok(())
        });
    }

    pub fn user_scroll(self: &Arc<Self>, agent: &str, dy: f64) {
        self.user_action(agent, move |_, tab| {
            tab.eval(&format!("window.scrollBy(0, {dy})"))?;
            Ok(())
        });
    }

    pub fn user_open(self: &Arc<Self>, agent: &str, url: String) {
        let a = agent.to_string();
        self.user_action(agent, move |me, tab| {
            let url = if url.contains("://") { url } else { format!("https://{url}") };
            if me.is_recording(&a) {
                me.record(&a, format!("Opened {url}"));
            }
            tab.call("Page.navigate", json!({"url": url}))?;
            wait_ready(tab);
            Ok(())
        });
    }

    /// Open the tab (starting the browser) in the background so the live view has something to show.
    pub fn warm_up(self: &Arc<Self>, agent: &str) {
        self.user_action(agent, |_, _| Ok(()));
    }
}

fn capture(tab: &Tab) -> Result<Frame, String> {
    let r = tab.call("Page.captureScreenshot", json!({"format": "jpeg", "quality": 60}))?;
    let data = r.get("data").and_then(Value::as_str).ok_or("no screenshot")?;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
    let info = tab.eval("JSON.stringify([location.href, document.title])").ok();
    let (url, title) = info.and_then(|v| v.as_str().and_then(|s| serde_json::from_str::<(String, String)>(s).ok())).unwrap_or_default();
    Ok(Frame { jpeg, url, title, seq: 0 })
}

fn wait_ready(tab: &Tab) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if tab.eval("document.readyState").ok().and_then(|v| v.as_str().map(str::to_string)).as_deref() == Some("complete") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn press(tab: &Tab, key: &str) -> Result<(), String> {
    let (code, vk, text) = match key {
        "Enter" => ("Enter", 13, "\r"),
        "Tab" => ("Tab", 9, ""),
        "Escape" => ("Escape", 27, ""),
        "Backspace" => ("Backspace", 8, ""),
        "ArrowDown" => ("ArrowDown", 40, ""),
        "ArrowUp" => ("ArrowUp", 38, ""),
        "ArrowLeft" => ("ArrowLeft", 37, ""),
        "ArrowRight" => ("ArrowRight", 39, ""),
        "PageDown" => ("PageDown", 34, ""),
        "PageUp" => ("PageUp", 33, ""),
        "Space" => ("Space", 32, " "),
        other => return Err(format!("Unsupported key \"{other}\". Use Enter, Tab, Escape, Backspace, arrows, PageUp, PageDown or Space.")),
    };
    let key_name = if key == "Space" { " " } else { key };
    let mut down = json!({"type": "keyDown", "key": key_name, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk});
    if !text.is_empty() {
        down["text"] = json!(text);
    }
    tab.call("Input.dispatchKeyEvent", down)?;
    tab.call(
        "Input.dispatchKeyEvent",
        json!({"type": "keyUp", "key": key_name, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk}),
    )?;
    Ok(())
}

fn running_as_root() -> bool {
    cfg!(unix) && (std::env::var("USER").is_ok_and(|u| u == "root") || std::env::var("HOME").is_ok_and(|h| h == "/root"))
}

/// Find a Chromium-based browser: the configured path, `FREEBOT_BROWSER`, then the usual places.
pub fn find_browser(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(PathBuf::from(configured));
    }
    if let Ok(p) = std::env::var("FREEBOT_BROWSER") {
        return Some(PathBuf::from(p));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in ["Google Chrome", "Chromium", "Microsoft Edge", "Brave Browser"] {
            candidates.push(PathBuf::from(format!("/Applications/{app}.app/Contents/MacOS/{app}")));
        }
    } else if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
            if let Ok(b) = std::env::var(base) {
                candidates.push(PathBuf::from(&b).join("Google\\Chrome\\Application\\chrome.exe"));
                candidates.push(PathBuf::from(&b).join("Microsoft\\Edge\\Application\\msedge.exe"));
                candidates.push(PathBuf::from(&b).join("Chromium\\Application\\chrome.exe"));
            }
        }
    } else {
        let path = std::env::var_os("PATH").unwrap_or_default();
        for name in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "brave-browser"] {
            for dir in std::env::split_paths(&path) {
                candidates.push(dir.join(name));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

const SNAPSHOT_JS: &str = r#"(() => {
  const out = []; let n = 0;
  document.querySelectorAll('[data-fb-ref]').forEach(e => e.removeAttribute('data-fb-ref'));
  const vis = e => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
  const sel = 'a[href],button,input:not([type=hidden]),textarea,select,[role=button],[role=link],[role=checkbox],[role=tab],[role=menuitem],[role=option],[contenteditable=true],summary';
  const els = [...document.querySelectorAll(sel)].filter(vis).slice(0, 250);
  for (const e of els) {
    n++; e.setAttribute('data-fb-ref', n);
    const tag = e.tagName.toLowerCase(); const type = e.getAttribute('type') || '';
    const label = (e.getAttribute('aria-label') || e.innerText || (type !== 'password' ? e.value : '') || e.getAttribute('placeholder') || e.getAttribute('title') || e.getAttribute('name') || '').trim().replace(/\s+/g, ' ').slice(0, 80);
    let d = '[' + n + '] ' + tag + (type ? '(' + type + ')' : '') + ' "' + label + '"';
    if (tag === 'a') d += ' -> ' + (e.getAttribute('href') || '').slice(0, 100);
    if ((tag === 'input' || tag === 'textarea') && e.value && type !== 'password') d += ' value="' + e.value.slice(0, 60) + '"';
    if (e.checked) d += ' (checked)';
    out.push(d);
  }
  const text = document.body ? document.body.innerText.replace(/\n{3,}/g, '\n\n').slice(0, 12000) : '';
  return JSON.stringify({url: location.href, title: document.title, elements: out, text});
})()"#;
