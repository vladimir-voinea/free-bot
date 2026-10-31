//! Background mode (`free-bot --background`): runs bots, routines, webhooks and
//! file watches without a window, and the guard that keeps only one copy of
//! Free Bot working on a data folder at a time.
//!
//! The guard is a listener on a localhost port derived from the data folder.
//! Opening the app while background mode runs asks the background copy to
//! hand over (it saves and exits), so the window can take over its bots.

use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::agents::engine::Fleet;
use crate::api::Waker;
use crate::store::Store;

const HANDOVER: &[u8] = b"free-bot handover\n";

fn guard_port(root: &Path) -> u16 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf()).hash(&mut h);
    40_000 + (h.finish() % 20_000) as u16
}

/// Holds the data folder for this process until dropped.
pub struct InstanceGuard {
    listener: TcpListener,
}

impl InstanceGuard {
    fn try_bind(root: &Path) -> Option<Self> {
        TcpListener::bind(("127.0.0.1", guard_port(root))).ok().map(|listener| Self { listener })
    }

    /// For the app window: take the data folder, asking a background copy to
    /// hand over if one is running. `None` means another window has it.
    pub fn acquire_for_window(root: &Path) -> Option<Self> {
        if let Some(g) = Self::try_bind(root) {
            return Some(g);
        }
        if let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], guard_port(root)).into(), Duration::from_secs(1)) {
            let _ = s.write_all(HANDOVER);
        }
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(150));
            if let Some(g) = Self::try_bind(root) {
                return Some(g);
            }
        }
        None
    }

    /// True when another process asked us to hand over.
    fn handover_requested(&self) -> bool {
        let _ = self.listener.set_nonblocking(true);
        while let Ok((mut s, _)) = self.listener.accept() {
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = [0u8; 32];
            if let Ok(n) = s.read(&mut buf)
                && &buf[..n] == HANDOVER
            {
                return true;
            }
        }
        false
    }
}

struct PrintWaker;
impl Waker for PrintWaker {
    fn wake(&self) {}
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Runs until the app window takes over or the process is stopped.
pub fn run() -> i32 {
    let store = Store::open_default();
    let root = store.root().to_path_buf();
    let Some(guard) = InstanceGuard::try_bind(&root) else {
        eprintln!("Free Bot is already open for {}. Its bots run there.", root.display());
        return 1;
    };
    let mut settings = store.load_settings();
    let settings_path = root.join("settings.json");
    let mut settings_mtime = mtime(&settings_path);
    let fleet = Fleet::open(&root, settings.clone(), Arc::new(PrintWaker));
    let browser_path = fleet.lock().config.browser_path.clone();
    fleet.browser().set_path(&browser_path);
    {
        let st = fleet.lock();
        println!(
            "Free Bot is running in the background for {} ({} bots, {} routines). Open the app to take over.",
            root.display(),
            st.agents.len(),
            st.routines.iter().filter(|r| r.enabled).count()
        );
    }
    loop {
        if guard.handover_requested() {
            println!("The app was opened; handing over.");
            break;
        }
        // Pick up provider changes made while running.
        let m = mtime(&settings_path);
        if m != settings_mtime {
            settings_mtime = m;
            settings = store.load_settings();
        }
        fleet.set_settings(&settings);
        fleet.tick();
        let (notices, notify) = {
            let mut st = fleet.lock();
            (std::mem::take(&mut st.notices), st.config.notifications)
        };
        for n in notices {
            println!("[{}] {}: {}", chrono::Local::now().format("%H:%M"), n.title, n.body);
            if notify {
                let _ = notify_rust::Notification::new().summary(&n.title).body(&n.body).appname("Free Bot").show();
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    fleet.stop_all();
    // Give running turns a moment to record that they were stopped.
    std::thread::sleep(Duration::from_millis(600));
    fleet.flush();
    drop(guard);
    0
}
