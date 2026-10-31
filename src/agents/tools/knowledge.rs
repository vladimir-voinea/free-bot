//! Memory, the shared knowledge folder, skills and routines.

use std::path::Path;

use chrono::Utc;
use serde_json::Value;

use super::{Risk, ToolCtx, ToolDef, ToolOutput, schema, str_arg};
use crate::agents::model::{Memory, Routine, Skill, Trigger};
use crate::agents::prompt::short_id;
use crate::store::new_id;

pub fn memory_definitions(has_knowledge: bool) -> Vec<ToolDef> {
    let mut v = vec![
        ToolDef::new(
            "remember",
            "Save a lasting fact or preference so you (or the whole team) remember it in future conversations.",
            schema(&[
                ("fact", "string", "One short fact, written so it makes sense later", true),
                ("shared", "boolean", "true to share it with every agent on the team", false),
            ]),
            Risk::Internal,
            "Remembering",
        ),
        ToolDef::new(
            "forget",
            "Delete a memory that is wrong or outdated.",
            schema(&[("memory_id", "string", "The id shown in brackets", true)]),
            Risk::Internal,
            "Forgetting",
        ),
        ToolDef::new(
            "search_memory",
            "Search everything you and the team remember.",
            schema(&[("query", "string", "Words to look for", true)]),
            Risk::Internal,
            "Searching memory",
        ),
    ];
    if has_knowledge {
        v.push(ToolDef::new(
            "search_knowledge",
            "Search the team's knowledge folder (notes and documents) and get matching passages with file paths. Read a whole file with read_file using the path shown.",
            schema(&[("query", "string", "Words to look for", true)]),
            Risk::Internal,
            "Searching the knowledge folder",
        ));
    }
    v
}

pub fn skill_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef::new(
            "use_skill",
            "Load the step-by-step instructions of a saved skill before doing that kind of task.",
            schema(&[("name", "string", "Skill name", true)]),
            Risk::Internal,
            "Loading a skill",
        ),
        ToolDef::new(
            "save_skill",
            "Save a repeatable procedure as a skill, or update an existing one after corrections. Any agent can then load it.",
            schema(&[
                ("name", "string", "Short name, for example weekly-invoice-report", true),
                ("description", "string", "When to use it, one sentence", true),
                ("instructions", "string", "Numbered steps, specific enough to repeat without help", true),
            ]),
            Risk::Internal,
            "Saving a skill",
        ),
    ]
}

pub fn routine_definitions() -> Vec<ToolDef> {
    vec![ToolDef::new(
        "create_routine",
        "Schedule recurring work for yourself. It runs on its own and posts results in your direct chat.",
        schema(&[
            ("name", "string", "Short name", true),
            (
                "schedule",
                "string",
                "When to run: \"every 30 minutes\", \"hourly at :15\", \"daily at 09:00\", \"weekly on monday at 9:00\" or \"monthly on day 1 at 8:00\"",
                true,
            ),
            ("instructions", "string", "What to do each time", true),
        ]),
        Risk::Safe,
        "Scheduling a routine",
    )]
}

pub fn execute(ctx: &ToolCtx, name: &str, args: &Value) -> Option<ToolOutput> {
    Some(match name {
        "remember" => remember(ctx, args),
        "forget" => forget(ctx, args),
        "search_memory" => search_memory(ctx, args),
        "search_knowledge" => search_knowledge(ctx, args),
        "use_skill" => use_skill(ctx, args),
        "save_skill" => save_skill(ctx, args),
        "create_routine" => create_routine(ctx, args),
        _ => return None,
    })
}

fn remember(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(fact) = str_arg(args, "fact") else { return ToolOutput::err("The fact is empty.") };
    let shared = args.get("shared").and_then(Value::as_bool).unwrap_or(false);
    let mut st = ctx.fleet.lock();
    if st.memories.iter().any(|m| m.text.eq_ignore_ascii_case(fact)) {
        return ToolOutput::ok("Already remembered.");
    }
    let m =
        Memory { id: new_id(), agent: if shared { None } else { Some(ctx.agent.id.clone()) }, text: fact.to_string(), created: Utc::now() };
    let id = short_id(&m.id).to_string();
    st.memories.push(m);
    st.dirty.memories = true;
    ToolOutput::ok(format!("Remembered [{id}]{}.", if shared { " for the whole team" } else { "" }))
}

fn forget(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(id) = str_arg(args, "memory_id") else { return ToolOutput::err("Give the memory id.") };
    let id = id.trim_matches(|c| c == '[' || c == ']');
    let mut st = ctx.fleet.lock();
    let before = st.memories.len();
    let me = ctx.agent.id.clone();
    st.memories.retain(|m| !(m.id.starts_with(id) && (m.agent.is_none() || m.agent.as_deref() == Some(&me))));
    if st.memories.len() == before {
        return ToolOutput::err("No memory with that id.");
    }
    st.dirty.memories = true;
    ToolOutput::ok("Forgotten.")
}

fn score(text: &str, terms: &[String]) -> usize {
    let lower = text.to_lowercase();
    terms.iter().map(|t| lower.matches(t.as_str()).count()).sum()
}

fn terms(q: &str) -> Vec<String> {
    q.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(str::to_string).collect()
}

fn search_memory(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let q = str_arg(args, "query").unwrap_or("");
    let t = terms(q);
    let st = ctx.fleet.lock();
    let mut hits: Vec<(usize, &Memory)> = st
        .memories
        .iter()
        .filter(|m| m.agent.is_none() || m.agent.as_deref() == Some(&ctx.agent.id))
        .map(|m| (score(&m.text, &t), m))
        .filter(|(s, _)| *s > 0 || t.is_empty())
        .collect();
    hits.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    if hits.is_empty() {
        return ToolOutput::ok("Nothing found.");
    }
    ToolOutput::ok(hits.iter().take(20).map(|(_, m)| format!("- [{}] {}", short_id(&m.id), m.text)).collect::<Vec<_>>().join("\n"))
}

fn search_knowledge(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(dir) = ctx.fleet.lock().config.knowledge_dir.clone() else { return ToolOutput::err("No knowledge folder is set.") };
    let q = str_arg(args, "query").unwrap_or("");
    let t = terms(q);
    if t.is_empty() {
        return ToolOutput::err("Give some words to search for.");
    }
    let mut hits: Vec<(usize, String, String)> = Vec::new();
    let mut files = Vec::new();
    collect_text_files(&dir, &mut files, 0);
    for f in files.iter().take(3000) {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        // Score paragraphs so results point at the useful part of long files.
        for para in text.split("\n\n") {
            let s = score(para, &t);
            if s > 0 {
                let rel = f.strip_prefix(&dir).unwrap_or(f).display().to_string();
                let snippet: String = para.trim().chars().take(600).collect();
                hits.push((s, rel, snippet));
            }
        }
    }
    hits.sort_by_key(|(s, _, _)| std::cmp::Reverse(*s));
    if hits.is_empty() {
        return ToolOutput::ok("Nothing found in the knowledge folder.");
    }
    let out: Vec<String> = hits.iter().take(8).map(|(_, p, s)| format!("### knowledge/{p}\n{s}")).collect();
    ToolOutput::ok(out.join("\n\n"))
}

pub fn collect_text_files(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        if p.is_dir() {
            collect_text_files(&p, out, depth + 1);
        } else if let Some(ext) = p.extension().and_then(|x| x.to_str())
            && matches!(ext.to_lowercase().as_str(), "md" | "txt" | "markdown" | "org" | "csv" | "json" | "yaml" | "yml" | "html" | "rst")
            && e.metadata().is_ok_and(|m| m.len() < 2_000_000)
        {
            out.push(p);
        }
    }
}

fn use_skill(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let Some(name) = str_arg(args, "name") else { return ToolOutput::err("Give the skill name.") };
    let st = ctx.fleet.lock();
    match st.skills.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
        Some(s) => ToolOutput::ok(format!("# Skill: {}\n{}\n\n{}", s.name, s.description, s.instructions)),
        None => {
            let names: Vec<&str> = st.skills.iter().map(|s| s.name.as_str()).collect();
            ToolOutput::err(format!(
                "No skill called \"{name}\". Saved skills: {}.",
                if names.is_empty() { "none".into() } else { names.join(", ") }
            ))
        }
    }
}

fn save_skill(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let (Some(name), Some(instr)) = (str_arg(args, "name"), str_arg(args, "instructions")) else {
        return ToolOutput::err("Give a name and instructions.");
    };
    let desc = str_arg(args, "description").unwrap_or("").to_string();
    let mut st = ctx.fleet.lock();
    st.dirty.skills = true;
    if let Some(s) = st.skills.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
        s.instructions = instr.to_string();
        if !desc.is_empty() {
            s.description = desc;
        }
        s.updated = Utc::now();
        return ToolOutput::ok(format!("Updated the skill \"{name}\"."));
    }
    st.skills.push(Skill {
        id: new_id(),
        name: name.to_string(),
        description: desc,
        instructions: instr.to_string(),
        author: Some(ctx.agent.id.clone()),
        updated: Utc::now(),
    });
    ToolOutput::ok(format!("Saved the skill \"{name}\"."))
}

fn create_routine(ctx: &ToolCtx, args: &Value) -> ToolOutput {
    let (Some(name), Some(sched), Some(instr)) = (str_arg(args, "name"), str_arg(args, "schedule"), str_arg(args, "instructions")) else {
        return ToolOutput::err("Give a name, a schedule and instructions.");
    };
    let Some(trigger) = Trigger::parse(sched) else {
        return ToolOutput::err("I could not read that schedule. Try \"daily at 09:00\" or \"every 2 hours\".");
    };
    let mut r = Routine::new(name.to_string(), ctx.agent.id.clone(), instr.to_string(), trigger);
    // Results go where the routine was set up, so the user sees them in context.
    r.thread = Some(ctx.thread.to_string());
    let next = r.next_run().map(|t| t.format("%a %b %-d %H:%M").to_string()).unwrap_or_default();
    let desc = r.trigger.describe();
    let mut st = ctx.fleet.lock();
    st.routines.push(r);
    st.dirty.routines = true;
    ToolOutput::ok(format!("Created the routine \"{name}\" ({desc}). Next run: {next}. The user can edit it under Routines."))
}
