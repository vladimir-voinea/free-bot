//! The agent's own computer: a workspace folder with files, and a terminal.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Risk, ToolCtx, ToolDef, ToolOutput, schema, str_arg};

const MAX_READ: usize = 100_000;
const MAX_OUTPUT: usize = 30_000;

pub fn file_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef::new(
            "list_files",
            "List files and folders in your workspace. Paths are relative to the workspace; use knowledge/... for the team's knowledge folder.",
            schema(&[("path", "string", "Folder to list; empty for the workspace root", false)]),
            Risk::Safe,
            "Looking at files",
        ),
        ToolDef::new(
            "read_file",
            "Read a text file from your workspace (or knowledge/... from the knowledge folder).",
            schema(&[("path", "string", "File path", true)]),
            Risk::Safe,
            "Reading a file",
        ),
        ToolDef::new(
            "write_file",
            "Create or overwrite a text file in your workspace. Folders are created as needed.",
            schema(&[("path", "string", "File path", true), ("content", "string", "Full file content", true)]),
            Risk::Safe,
            "Writing a file",
        ),
    ]
}

pub fn terminal_definitions() -> Vec<ToolDef> {
    vec![ToolDef::new(
        "run_command",
        &format!(
            "Run a shell command in your workspace folder ({}) and get its output. Use it to run scripts, code, git, and command-line tools.",
            if cfg!(windows) { "cmd.exe" } else { "sh" }
        ),
        schema(&[
            ("command", "string", "The command line", true),
            ("timeout_secs", "integer", "Seconds before it is stopped (default 120, max 900)", false),
        ]),
        Risk::Risky,
        "Running a command",
    )]
}

pub fn execute(ctx: &ToolCtx, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "list_files" => list_files(ctx, args),
        "read_file" => read_file(ctx, args),
        "write_file" => write_file(ctx, args),
        "run_command" => run_command(ctx, args),
        _ => return None,
    })
}

/// Resolve a path the model gave into a real path, refusing anything outside
/// the workspace (or the knowledge folder for reads).
pub fn resolve(ctx: &ToolCtx, raw: &str, write: bool) -> Result<PathBuf, String> {
    let ws = ctx.fleet.workspace_of(ctx.agent);
    let knowledge = ctx.fleet.lock().config.knowledge_dir.clone();
    let raw = raw.trim();
    let (base, rel) = if let Some(rest) = raw.strip_prefix("knowledge/").or_else(|| (raw == "knowledge").then_some("")) {
        if write {
            return Err("The knowledge folder is read-only.".into());
        }
        let Some(k) = knowledge else { return Err("No knowledge folder is set.".into()) };
        (k, rest.to_string())
    } else {
        let p = Path::new(raw);
        if p.is_absolute() {
            if let Ok(rel) = p.strip_prefix(&ws) {
                (ws.clone(), rel.display().to_string())
            } else if let Some(k) = knowledge.as_ref().filter(|k| !write && p.starts_with(k)) {
                (k.clone(), p.strip_prefix(k).unwrap_or(p).display().to_string())
            } else {
                return Err(format!("Only paths inside your workspace are allowed ({}).", ws.display()));
            }
        } else {
            (ws.clone(), raw.to_string())
        }
    };
    let mut out = base.clone();
    for c in Path::new(&rel).components() {
        match c {
            Component::Normal(x) => out.push(x),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() || !out.starts_with(&base) {
                    return Err("Paths cannot leave the workspace.".into());
                }
            }
            _ => return Err("Use a relative path.".into()),
        }
    }
    if !out.starts_with(&base) {
        return Err("Paths cannot leave the workspace.".into());
    }
    // Symlinks could point outside; check the real location when it exists.
    if let (Ok(real), Ok(real_base)) = (out.canonicalize(), base.canonicalize())
        && !real.starts_with(&real_base)
    {
        return Err("That path points outside the workspace.".into());
    }
    Ok(out)
}

fn list_files(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let path = match resolve(ctx, str_arg(args, "path").unwrap_or(""), false) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    let Ok(rd) = std::fs::read_dir(&path) else { return ToolOutput::err(format!("Cannot open {}.", path.display())) };
    let mut entries: Vec<(bool, String, u64)> = rd
        .flatten()
        .map(|e| {
            let m = e.metadata().ok();
            (m.as_ref().is_some_and(|m| m.is_dir()), e.file_name().to_string_lossy().to_string(), m.map(|m| m.len()).unwrap_or(0))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    if entries.is_empty() {
        return ToolOutput::ok("(empty folder)");
    }
    let lines: Vec<String> =
        entries.iter().take(500).map(|(d, n, s)| if *d { format!("{n}/") } else { format!("{n}  ({})", human_size(*s)) }).collect();
    ToolOutput::ok(lines.join("\n"))
}

pub fn human_size(b: u64) -> String {
    match b {
        0..1024 => format!("{b} B"),
        1024..1_048_576 => format!("{:.1} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

fn read_file(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(raw) = str_arg(args, "path") else { return ToolOutput::err("Give a path.") };
    let path = match resolve(ctx, raw, false) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    match std::fs::read(&path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => ToolOutput::ok(crate::agents::prompt::truncate(&s, MAX_READ)),
            Err(_) => ToolOutput::err("That is a binary file, not text."),
        },
        Err(e) => ToolOutput::err(format!("Cannot read {raw}: {e}")),
    }
}

fn write_file(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(raw) = str_arg(args, "path") else { return ToolOutput::err("Give a path.") };
    let content = args.get("content").and_then(Value::as_str).unwrap_or("");
    let path = match resolve(ctx, raw, true) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, content) {
        Ok(()) => ToolOutput::ok(format!("Wrote {raw} ({}).", human_size(content.len() as u64))),
        Err(e) => ToolOutput::err(format!("Cannot write {raw}: {e}")),
    }
}

fn run_command(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(cmd) = str_arg(args, "command") else { return ToolOutput::err("Give a command.") };
    let timeout = Duration::from_secs(args.get("timeout_secs").and_then(Value::as_u64).unwrap_or(120).clamp(1, 900));
    let ws = ctx.fleet.workspace_of(ctx.agent);
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    };
    command.current_dir(&ws).env("FREEBOT_WORKSPACE", &ws).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return ToolOutput::err(format!("Could not start the command: {e}")),
    };
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let t_out = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let t_err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if ctx.cancel.load(Ordering::Relaxed) || started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    let stdout = String::from_utf8_lossy(&t_out.join().unwrap_or_default()).to_string();
    let stderr = String::from_utf8_lossy(&t_err.join().unwrap_or_default()).to_string();
    let mut text = String::new();
    match status {
        Some(s) => text.push_str(&format!("Exit code: {}\n", s.code().map(|c| c.to_string()).unwrap_or_else(|| "killed".into()))),
        None if ctx.cancel.load(Ordering::Relaxed) => text.push_str("Stopped.\n"),
        None => text.push_str(&format!("Timed out after {}s and was stopped.\n", timeout.as_secs())),
    }
    if !stdout.trim().is_empty() {
        text.push_str(&format!("\n{}", tail(&stdout, MAX_OUTPUT)));
    }
    if !stderr.trim().is_empty() {
        text.push_str(&format!("\n[stderr]\n{}", tail(&stderr, MAX_OUTPUT / 3)));
    }
    let failed = status.is_none_or(|s| !s.success());
    ToolOutput { text, error: failed, ..Default::default() }
}

/// Keep the end of long output, where errors and results usually are.
fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("… ({} earlier characters cut)\n{}", start, &s[start..])
}
