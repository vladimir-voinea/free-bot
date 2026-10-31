//! On-disk storage for the agent layer, under `<data dir>/fleet/`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::model::{Agent, FleetConfig, Memory, Routine, Skill, Task, Thread};
use crate::store::write_atomic;

pub struct FleetStore {
    root: PathBuf,
}

impl FleetStore {
    pub fn open(data_root: &Path) -> Self {
        let root = data_root.join("fleet");
        let _ = fs::create_dir_all(root.join("threads"));
        let _ = fs::create_dir_all(root.join("agents"));
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn browser_profile_dir(&self) -> PathBuf {
        self.root.join("browser-profile")
    }

    fn load<T: DeserializeOwned + Default>(&self, name: &str) -> T {
        fs::read(self.root.join(name)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save<T: Serialize>(&self, name: &str, value: &T, private: bool) {
        match serde_json::to_vec_pretty(value) {
            Ok(bytes) => {
                if let Err(e) = write_atomic(&self.root.join(name), &bytes, private) {
                    eprintln!("failed to save {name}: {e}");
                }
            }
            Err(e) => eprintln!("failed to encode {name}: {e}"),
        }
    }

    pub fn load_agents(&self) -> Vec<Agent> {
        self.load("agents.json")
    }
    pub fn save_agents(&self, v: &[Agent]) {
        self.save("agents.json", &v, false)
    }
    pub fn load_memories(&self) -> Vec<Memory> {
        self.load("memory.json")
    }
    pub fn save_memories(&self, v: &[Memory]) {
        self.save("memory.json", &v, false)
    }
    pub fn load_skills(&self) -> Vec<Skill> {
        self.load("skills.json")
    }
    pub fn save_skills(&self, v: &[Skill]) {
        self.save("skills.json", &v, false)
    }
    pub fn load_tasks(&self) -> Vec<Task> {
        self.load("tasks.json")
    }
    pub fn save_tasks(&self, v: &[Task]) {
        self.save("tasks.json", &v, false)
    }
    pub fn load_routines(&self) -> Vec<Routine> {
        self.load("routines.json")
    }
    pub fn save_routines(&self, v: &[Routine]) {
        self.save("routines.json", &v, false)
    }
    pub fn load_config(&self) -> FleetConfig {
        self.load("config.json")
    }
    /// Connector settings can hold tokens, so the file is private to the user.
    pub fn save_config(&self, v: &FleetConfig) {
        self.save("config.json", v, true)
    }

    pub fn load_threads(&self) -> Vec<Thread> {
        let mut out = Vec::new();
        if let Ok(rd) = fs::read_dir(self.root.join("threads")) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "json")
                    && let Some(t) = fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Thread>(&b).ok())
                {
                    out.push(t);
                }
            }
        }
        out.sort_by_key(|t| t.created);
        out
    }

    pub fn save_thread(&self, t: &Thread) {
        self.save(&format!("threads/{}.json", safe(&t.id)), t, false)
    }

    pub fn delete_thread(&self, id: &str) {
        let _ = fs::remove_file(self.root.join("threads").join(format!("{}.json", safe(id))));
    }
}

fn safe(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}
