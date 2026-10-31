//! Settings dialog: general preferences, providers and models,
//! personalization and data controls.

use egui::{Align, Align2, CornerRadius, Layout, Sense, Stroke, Vec2, pos2, vec2};

use super::icons::{self, Icon};
use super::sidebar::menu_item;
use super::theme::{self, Palette};
use super::widgets;
use crate::api;
use crate::app::{FreeBotApp, SettingsTab};
use crate::store::{ModelRef, PROVIDER_PRESETS, Provider, ThemePref};

const TABS: &[(SettingsTab, Icon, &str)] = &[
    (SettingsTab::General, Icon::Gear, "General"),
    (SettingsTab::Providers, Icon::Key, "Providers & models"),
    (SettingsTab::Personalize, Icon::Pencil, "Personalize"),
    (SettingsTab::Bots, Icon::Bot, "Bots"),
    (SettingsTab::Connectors, Icon::Plug, "Connectors"),
    (SettingsTab::Data, Icon::Download, "Data controls"),
    (SettingsTab::About, Icon::Chat, "About"),
];

impl FreeBotApp {
    pub fn settings_modal(&mut self, ctx: &egui::Context) {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        self.poll_model_fetch();
        let screen = ctx.content_rect();
        let size = vec2(820.0f32.min(screen.width() - 40.0), 600.0f32.min(screen.height() - 40.0));
        let mut close = false;

        let modal = egui::Modal::new(egui::Id::unique("settings-modal")).frame(Self::settings_frame(ctx)).backdrop_color(p.overlay).show(
            ctx,
            |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                ui.horizontal_top(|ui| {
                    // Tab column.
                    let (col, _) = ui.allocate_exact_size(vec2(210.0, size.y), Sense::hover());
                    ui.painter().rect_filled(col, CornerRadius { nw: 20, sw: 20, ne: 0, se: 0 }, p.sidebar);
                    let mut tabs =
                        ui.new_child(egui::UiBuilder::new().max_rect(col.shrink2(vec2(10.0, 16.0))).layout(Layout::top_down(Align::LEFT)));
                    tabs.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("Settings").font(theme::font_bold(18.0)).color(p.text));
                    });
                    tabs.add_space(12.0);
                    let current = self.settings_ui.as_ref().map(|s| s.tab).unwrap_or(SettingsTab::General);
                    for (tab, icon, label) in TABS {
                        let (rect, resp) = tabs.allocate_exact_size(vec2(tabs.available_width(), 36.0), Sense::click());
                        if *tab == current {
                            tabs.painter().rect_filled(rect, CornerRadius::same(10), p.selected);
                        } else if resp.hovered() {
                            tabs.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
                        }
                        icons::paint(
                            tabs.painter(),
                            egui::Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(18.0)),
                            *icon,
                            p.text,
                        );
                        tabs.painter().text(
                            pos2(rect.left() + 38.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            *label,
                            theme::font(14.0),
                            p.text,
                        );
                        if resp.clicked()
                            && let Some(s) = &mut self.settings_ui
                        {
                            s.tab = *tab;
                        }
                    }

                    // Content.
                    let content_rect =
                        egui::Rect::from_min_max(pos2(col.right(), col.top()), pos2(col.right() + size.x - 210.0, col.bottom()));
                    let mut content = ui.new_child(
                        egui::UiBuilder::new().max_rect(content_rect.shrink2(vec2(26.0, 18.0))).layout(Layout::top_down(Align::LEFT)),
                    );
                    content.horizontal(|ui| {
                        let title = TABS.iter().find(|t| t.0 == current).map(|t| t.2).unwrap_or("");
                        ui.label(egui::RichText::new(title).font(theme::font_bold(18.0)).color(p.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::icon_button(ui, Icon::Close, "Close").clicked() {
                                close = true;
                            }
                        });
                    });
                    content.add_space(10.0);
                    egui::ScrollArea::vertical().id_salt(("settings", current as u8)).auto_shrink([false, false]).show(
                        &mut content,
                        |ui| {
                            ui.set_width(ui.available_width() - 8.0);
                            match current {
                                SettingsTab::General => self.general_tab(ui),
                                SettingsTab::Providers => self.providers_tab(ui),
                                SettingsTab::Personalize => self.personalize_tab(ui),
                                SettingsTab::Bots => self.bots_settings_tab(ui),
                                SettingsTab::Connectors => self.connectors_settings_tab(ui),
                                SettingsTab::Data => self.data_tab(ui),
                                SettingsTab::About => about_tab(ui),
                            }
                        },
                    );
                });
            },
        );
        if close || modal.should_close() {
            self.settings_ui = None;
            self.save_settings();
        }
    }

    fn settings_frame(ctx: &egui::Context) -> egui::Frame {
        let p = Palette::for_dark(ctx.global_style().visuals.dark_mode);
        egui::Frame::new().fill(p.surface).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(20)).shadow(egui::Shadow {
            offset: [0, 12],
            blur: 40,
            spread: 0,
            color: egui::Color32::from_black_alpha(90),
        })
    }

    fn general_tab(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let mut dirty = false;
        widgets::setting_row(ui, "Theme", "Choose how Free Bot looks.", |ui| {
            for (pref, icon, label) in [
                (ThemePref::System, Icon::Monitor, "System"),
                (ThemePref::Light, Icon::Sun, "Light"),
                (ThemePref::Dark, Icon::Moon, "Dark"),
            ]
            .iter()
            .rev()
            {
                let on = self.settings.theme == *pref;
                if widgets::pill_toggle(ui, *icon, label, on, "").clicked() {
                    self.settings.theme = *pref;
                    dirty = true;
                }
            }
        });
        widgets::setting_row(ui, "Your name", "Used in the greeting and shared with the model.", |ui| {
            ui.allocate_ui(vec2(200.0, 34.0), |ui| {
                if widgets::text_field(ui, &mut self.settings.display_name, "Name", false).changed() {
                    dirty = true;
                }
            });
        });
        widgets::setting_row(ui, "Enter to send", "When off, use Ctrl+Enter to send and Enter for a new line.", |ui| {
            dirty |= widgets::switch(ui, &mut self.settings.enter_to_send).changed();
        });
        widgets::setting_row(ui, "Auto-generate chat titles", "Ask the model for a short title after the first answer.", |ui| {
            dirty |= widgets::switch(ui, &mut self.settings.auto_title).changed();
        });
        widgets::setting_row(ui, "Show model name", "Show which model wrote each answer.", |ui| {
            dirty |= widgets::switch(ui, &mut self.settings.show_model_name).changed();
        });
        widgets::setting_row(ui, "Interface size", "Zoom the whole interface.", |ui| {
            let mut pct = (self.settings.ui_scale * 100.0).round() as i32;
            if ui.add(egui::Slider::new(&mut pct, 75..=150).suffix("%").step_by(5.0)).changed() {
                self.settings.ui_scale = pct as f32 / 100.0;
                dirty = true;
            }
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Keyboard shortcuts").font(theme::font_bold(14.5)).color(p.text));
        ui.add_space(6.0);
        let ctx = ui.ctx().clone();
        for (label, sc) in [
            ("New chat", super::SC_NEW_CHAT),
            ("Search chats", super::SC_SEARCH),
            ("Toggle sidebar", super::SC_SIDEBAR),
            ("Private chat", super::SC_PRIVATE),
            ("Settings", super::SC_SETTINGS),
        ] {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).color(p.muted).font(theme::font(13.5)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    kbd(ui, &super::shortcut_text(&ctx, &sc));
                });
            });
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Stop answering").color(p.muted).font(theme::font(13.5)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| kbd(ui, "Esc"));
        });
        if dirty {
            self.mark_settings_dirty();
        }
    }

    fn providers_tab(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Connect any OpenAI-compatible API. Your keys are stored only on this computer and sent only to the provider you configure.")
                    .color(p.muted)
                    .font(theme::font(13.5)),
            )
            .wrap(),
        );
        ui.add_space(12.0);

        // Default model.
        let models = self.settings.all_models();
        if !models.is_empty() {
            let current = self.settings.effective_model();
            widgets::setting_row(ui, "Default model", "Used for new prompts. You can switch in the prompt box too.", |ui| {
                let label = current.as_ref().map(|m| m.model.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt("default-model").selected_text(label).width(170.0).show_ui(ui, |ui| {
                    for (m, provider) in &models {
                        let sel = current.as_ref() == Some(m);
                        if ui.selectable_label(sel, format!("{}  ·  {}", m.model, provider)).clicked() {
                            self.settings.selected_model = Some(m.clone());
                        }
                    }
                });
            });
        }

        // Provider chooser row.
        let state_idx = self.settings_ui.as_ref().map(|s| s.provider_idx).unwrap_or(0);
        let idx = state_idx.min(self.settings.providers.len().saturating_sub(1));
        ui.horizontal_wrapped(|ui| {
            for (i, prov) in self.settings.providers.iter().enumerate() {
                let resp = widgets::pill_toggle(ui, Icon::Key, &prov.name, i == idx, &prov.base_url);
                if resp.clicked()
                    && let Some(s) = &mut self.settings_ui
                {
                    s.provider_idx = i;
                    s.fetch_status = None;
                }
            }
            let add = widgets::pill_button(ui, "Add provider", Some(Icon::Plus), true, true);
            let mut chosen = None;
            egui::Popup::menu(&add).width(240.0).show(|ui| {
                ui.set_min_width(230.0);
                for (i, (name, url, _)) in PROVIDER_PRESETS.iter().enumerate() {
                    let r = menu_item(ui, if url.is_empty() { Icon::Pencil } else { Icon::Plus }, name, false);
                    if r.clicked() {
                        chosen = Some(i);
                    }
                }
            });
            if let Some(i) = chosen {
                let (name, url, models) = PROVIDER_PRESETS[i];
                let prov = Provider {
                    name: if url.is_empty() { "Custom provider".into() } else { name.into() },
                    base_url: url.into(),
                    models: models.iter().map(|s| s.to_string()).collect(),
                    ..Default::default()
                };
                let first_model = prov.models.first().cloned();
                let pid = prov.id.clone();
                self.settings.providers.push(prov);
                if self.settings.selected_model.is_none()
                    && let Some(m) = first_model
                {
                    self.settings.selected_model = Some(ModelRef { provider_id: pid, model: m });
                }
                if let Some(s) = &mut self.settings_ui {
                    s.provider_idx = self.settings.providers.len() - 1;
                    s.fetch_status = None;
                }
                self.mark_settings_dirty();
            }
        });
        ui.add_space(14.0);

        if self.settings.providers.is_empty() {
            egui::Frame::new().fill(p.surface_hover).corner_radius(CornerRadius::same(14)).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new("No providers yet").font(theme::font_bold(15.0)).color(p.text));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("Click \"Add provider\" and pick a preset, or choose Custom for any server that speaks the OpenAI chat completions API.")
                            .color(p.muted)
                            .font(theme::font(13.5)),
                    )
                    .wrap(),
                );
            });
            return;
        }

        let mut delete = false;
        let mut fetch = None;
        let mut test = None;
        let st = self.settings_ui.as_mut().unwrap();
        let prov = &mut self.settings.providers[idx];
        let mut dirty = false;
        egui::Frame::new()
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                field_label(ui, "Name");
                dirty |= widgets::text_field(ui, &mut prov.name, "My provider", false).changed();
                ui.add_space(8.0);
                field_label(ui, "Base URL");
                dirty |= widgets::text_field(ui, &mut prov.base_url, "https://api.example.com/v1", false).changed();
                ui.label(egui::RichText::new("The URL that ends right before /chat/completions.").font(theme::font(12.0)).color(p.faint));
                ui.add_space(8.0);
                field_label(ui, "API key");
                ui.horizontal(|ui| {
                    ui.allocate_ui(vec2(ui.available_width() - 44.0, 34.0), |ui| {
                        dirty |= widgets::text_field(ui, &mut prov.api_key, "sk-… (leave empty for local servers)", !st.show_key).changed();
                    });
                    let icon = if st.show_key { Icon::Ghost } else { Icon::Key };
                    if widgets::icon_button(ui, icon, if st.show_key { "Hide key" } else { "Show key" }).clicked() {
                        st.show_key = !st.show_key;
                    }
                });
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    field_label(ui, "Models");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let busy = st.fetching.is_some();
                        if busy {
                            ui.spinner();
                        } else if widgets::secondary_button(ui, "Fetch models")
                            .on_hover_text("Load the model list from the provider (GET /models)")
                            .clicked()
                        {
                            fetch = Some(prov.clone());
                        }
                        if !busy && widgets::secondary_button(ui, "Test connection").clicked() {
                            test = Some(prov.clone());
                        }
                    });
                });
                if let Some(status) = &st.fetch_status {
                    let (text, color) = match status {
                        Ok(t) => (t.as_str(), p.muted),
                        Err(e) => (e.as_str(), p.danger),
                    };
                    ui.add(egui::Label::new(egui::RichText::new(text).font(theme::font(12.5)).color(color)).wrap());
                }
                ui.add_space(6.0);
                let mut remove = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, m) in prov.models.iter().enumerate() {
                        if model_chip(ui, m) {
                            remove = Some(i);
                        }
                    }
                });
                if let Some(i) = remove {
                    prov.models.remove(i);
                    dirty = true;
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let mut add = false;
                    ui.allocate_ui(vec2(ui.available_width() - 90.0, 34.0), |ui| {
                        let r = widgets::text_field(ui, &mut st.new_model, "Add a model id, e.g. gpt-4o-mini", false);
                        add = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    });
                    if widgets::secondary_button(ui, "Add").clicked() {
                        add = true;
                    }
                    let m = st.new_model.trim().to_string();
                    if add && !m.is_empty() {
                        if !prov.models.contains(&m) {
                            prov.models.push(m);
                        }
                        st.new_model.clear();
                        dirty = true;
                    }
                });
                ui.add_space(14.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::secondary_button(ui, "Remove provider").clicked() {
                        delete = true;
                    }
                });
            });

        if let Some(prov) = fetch {
            self.start_model_fetch(prov, true);
        }
        if let Some(prov) = test {
            self.start_model_fetch(prov, false);
        }
        if delete {
            let removed = self.settings.providers.remove(idx);
            if self.settings.selected_model.as_ref().is_some_and(|m| m.provider_id == removed.id) {
                self.settings.selected_model = None;
            }
            if let Some(s) = &mut self.settings_ui {
                s.provider_idx = idx.saturating_sub(1);
                s.fetch_status = None;
            }
            dirty = true;
        }
        if dirty {
            self.mark_settings_dirty();
        }
    }

    /// Fetch `/models` in the background. With `apply`, the list replaces the provider's models.
    fn start_model_fetch(&mut self, prov: Provider, apply: bool) {
        let (tx, rx) = std::sync::mpsc::channel();
        let waker = self.waker.clone();
        let id = if apply { prov.id.clone() } else { format!("test:{}", prov.id) };
        std::thread::spawn(move || {
            let res = api::fetch_models(&prov);
            let _ = tx.send((id, res));
            waker.wake();
        });
        if let Some(s) = &mut self.settings_ui {
            s.fetching = Some(rx);
            s.fetch_status = None;
        }
    }

    fn poll_model_fetch(&mut self) {
        let Some(s) = &mut self.settings_ui else { return };
        let Some(rx) = &s.fetching else { return };
        let Ok((id, res)) = rx.try_recv() else { return };
        s.fetching = None;
        match (id.strip_prefix("test:"), res) {
            (Some(_), Ok(models)) => s.fetch_status = Some(Ok(format!("Connected. The provider offers {} models.", models.len()))),
            (Some(_), Err(e)) | (None, Err(e)) => s.fetch_status = Some(Err(e)),
            (None, Ok(models)) => {
                let n = models.len();
                s.fetch_status = Some(Ok(format!("Loaded {n} models.")));
                if let Some(prov) = self.settings.providers.iter_mut().find(|p| p.id == id)
                    && n > 0
                {
                    prov.models = models;
                }
                self.mark_settings_dirty();
            }
        }
    }

    fn personalize_tab(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let mut dirty = false;
        ui.label(egui::RichText::new("Custom instructions").font(theme::font_bold(14.5)).color(p.text));
        ui.label(
            egui::RichText::new("What should Free Bot know about you, and how should it respond?").font(theme::font(13.0)).color(p.muted),
        );
        ui.add_space(6.0);
        widgets::input_frame(ui).show(ui, |ui| {
            dirty |= ui
                .add(
                    egui::TextEdit::multiline(&mut self.settings.custom_instructions)
                        .hint_text("For example: I'm a software engineer. Keep answers short and show code first.")
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                )
                .changed();
        });
        ui.add_space(16.0);

        let mut custom_temp = self.settings.temperature.is_some();
        widgets::setting_row(
            ui,
            "Custom temperature",
            "Lower is more focused, higher is more creative. Off uses the provider default.",
            |ui| {
                if widgets::switch(ui, &mut custom_temp).changed() {
                    self.settings.temperature = if custom_temp { Some(0.7) } else { None };
                    dirty = true;
                }
                if let Some(t) = &mut self.settings.temperature {
                    dirty |= ui.add(egui::Slider::new(t, 0.0..=2.0).step_by(0.05)).changed();
                }
            },
        );
        let mut custom_max = self.settings.max_tokens.is_some();
        widgets::setting_row(ui, "Max answer length", "Limit tokens per answer. Off uses the provider default.", |ui| {
            if widgets::switch(ui, &mut custom_max).changed() {
                self.settings.max_tokens = if custom_max { Some(4096) } else { None };
                dirty = true;
            }
            if let Some(m) = &mut self.settings.max_tokens {
                dirty |= ui.add(egui::DragValue::new(m).range(64..=200_000).speed(64.0)).changed();
            }
        });
        if dirty {
            self.mark_settings_dirty();
        }
    }

    fn data_tab(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let root = self.store.root().display().to_string();
        widgets::setting_row(ui, "Data folder", &root, |ui| {
            if widgets::secondary_button(ui, "Copy path").clicked() {
                ui.ctx().copy_text(root.clone());
            }
        });
        let n = self.convs.len();
        let mut export = false;
        widgets::setting_row(ui, "Export chats", &format!("Save all {n} chats as a JSON file."), |ui| {
            if widgets::secondary_button(ui, "Export").clicked() {
                export = true;
            }
        });
        if export {
            self.export_all_json();
        }
        let confirming = self.settings_ui.as_ref().is_some_and(|s| s.confirm_delete_all);
        let mut action = None;
        widgets::setting_row(ui, "Delete all chats", "Permanently delete every saved chat. This cannot be undone.", |ui| {
            if confirming {
                if widgets::danger_button(ui, "Delete everything").clicked() {
                    action = Some(true);
                }
                if widgets::secondary_button(ui, "Cancel").clicked() {
                    action = Some(false);
                }
            } else if ui
                .add(
                    egui::Button::new(egui::RichText::new("Delete all").color(p.danger))
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, p.danger))
                        .corner_radius(CornerRadius::same(18))
                        .min_size(vec2(0.0, 34.0)),
                )
                .clicked()
                && let Some(s) = &mut self.settings_ui
            {
                s.confirm_delete_all = true;
            }
        });
        match action {
            Some(true) => {
                self.delete_all_convs();
                if let Some(s) = &mut self.settings_ui {
                    s.confirm_delete_all = false;
                }
                self.toast("All chats deleted");
            }
            Some(false) => {
                if let Some(s) = &mut self.settings_ui {
                    s.confirm_delete_all = false;
                }
            }
            None => {}
        }
    }
}

fn about_tab(ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(18), p.accent);
        icons::paint_logo(ui.painter(), r.shrink(12.0), p.on_accent, p.accent);
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Free Bot").font(theme::font_bold(24.0)).color(p.text));
        ui.label(egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(p.muted));
        ui.add_space(10.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("A fast, native AI chat app written in Rust. Bring your own provider and model: Free Bot talks to any OpenAI-compatible API, keeps your chats on your computer, and never phones home.")
                    .color(p.muted)
                    .font(theme::font(14.0)),
            )
            .wrap(),
        );
        ui.add_space(10.0);
        ui.hyperlink_to("github.com/vladimir-voinea/free-bot", "https://github.com/vladimir-voinea/free-bot");
        ui.add_space(6.0);
        ui.label(egui::RichText::new("MIT licensed. Inter and JetBrains Mono fonts under their own open licenses.").font(theme::font(12.0)).color(p.faint));
    });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    let p = Palette::of(ui);
    ui.label(egui::RichText::new(text).font(theme::font(13.0)).color(p.muted));
}

fn kbd(ui: &mut egui::Ui, text: &str) {
    let p = Palette::of(ui);
    egui::Frame::new()
        .fill(p.surface_hover)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).font(theme::font(12.5)).color(p.text));
        });
}

/// A model id chip with a remove button. Returns true when remove is clicked.
fn model_chip(ui: &mut egui::Ui, name: &str) -> bool {
    let p = Palette::of(ui);
    let font = theme::font(13.0);
    let tw = ui.painter().layout_no_wrap(name.to_string(), font.clone(), p.text).size().x.min(260.0);
    let (rect, _) = ui.allocate_exact_size(vec2(tw + 44.0, 30.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(15), p.surface_hover, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    let t = widgets::truncate_to_width(ui, name, &font, tw);
    ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, t, font, p.text);
    let x = egui::Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), Vec2::splat(20.0));
    let r = ui.interact(x, ui.scope_id().with(("rm-model", name)), Sense::click());
    if r.hovered() {
        ui.painter().circle_filled(x.center(), 10.0, p.selected);
    }
    icons::paint(ui.painter(), x.shrink(5.0), Icon::Close, p.muted);
    r.on_hover_text("Remove model").clicked()
}
