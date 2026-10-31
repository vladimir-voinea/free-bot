#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod agents;
mod api;
mod app;
mod store;
mod ui;

/// Automated screenshot run, driven by `FREEBOT_SCREENSHOT` (output path) and
/// `FREEBOT_SCREENSHOT_VIEW` (home, chat, settings, search, rail, light…).
/// Used for docs and visual checks.
pub struct ScreenshotPlan {
    pub path: std::path::PathBuf,
    pub view: String,
    pub frames: u32,
    pub requested: bool,
    /// Prompt to send automatically before taking the screenshot (`FREEBOT_SCREENSHOT_SEND`).
    pub autosend: Option<String>,
    /// How long to wait before the screenshot (`FREEBOT_SCREENSHOT_DELAY_MS`).
    pub delay: std::time::Duration,
    pub started: std::time::Instant,
}

fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() -> eframe::Result {
    let screenshot = std::env::var_os("FREEBOT_SCREENSHOT").map(|p| ScreenshotPlan {
        path: p.into(),
        view: std::env::var("FREEBOT_SCREENSHOT_VIEW").unwrap_or_else(|_| "home".into()),
        frames: 0,
        requested: false,
        autosend: std::env::var("FREEBOT_SCREENSHOT_SEND").ok(),
        delay: std::time::Duration::from_millis(
            std::env::var("FREEBOT_SCREENSHOT_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(800),
        ),
        started: std::time::Instant::now(),
    });

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Free Bot")
        .with_app_id("free-bot")
        .with_inner_size([1240.0, 820.0])
        .with_min_inner_size([720.0, 520.0]);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Free Bot", options, Box::new(|cc| Ok(Box::new(app::FreeBotApp::new(cc, screenshot)))))
}
