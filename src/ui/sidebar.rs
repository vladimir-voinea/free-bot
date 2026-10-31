//! Left sidebar: brand, search, new chat, chat history and the profile row.
//! Collapses to a narrow icon rail.

use egui::{Align2, CornerRadius, Sense, Stroke, Vec2, pos2, vec2};

use super::icons::{self, Icon};
use super::theme::{self, Palette};
use super::widgets;
use super::{SC_NEW_CHAT, SC_SEARCH, shortcut_text};
use crate::app::{Confirm, FreeBotApp, SettingsTab};
use crate::store::DateGroup;

impl FreeBotApp {
    pub fn sidebar_ui(&mut self, ui: &mut egui::Ui) {
        if self.settings.sidebar_open {
            self.sidebar_expanded(ui);
        } else {
            self.sidebar_rail(ui);
        }
    }

    fn sidebar_rail(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        ui.vertical_centered(|ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
                icons::paint(ui.painter(), rect.shrink(8.0), Icon::Sidebar, p.text);
            } else {
                icons::paint_logo(ui.painter(), rect.shrink(5.0), p.text, p.sidebar);
            }
            if resp.on_hover_text("Open sidebar").clicked() {
                self.settings.sidebar_open = true;
                self.mark_settings_dirty();
            }
            ui.add_space(14.0);
            if widgets::icon_button_sized(ui, Icon::Search, &format!("Search  {}", shortcut_text(ui.ctx(), &SC_SEARCH)), 36.0, Some(p.text))
                .clicked()
            {
                self.open_search();
            }
            ui.add_space(4.0);
            if widgets::icon_button_sized(
                ui,
                Icon::Compose,
                &format!("New chat  {}", shortcut_text(ui.ctx(), &SC_NEW_CHAT)),
                36.0,
                Some(p.text),
            )
            .clicked()
            {
                self.new_chat();
            }
            ui.add_space(4.0);
            if widgets::icon_button_sized(ui, Icon::Chat, "History", 36.0, Some(p.text)).clicked() {
                self.settings.sidebar_open = true;
                self.mark_settings_dirty();
            }
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            if self.avatar(ui, 32.0).on_hover_text("Settings").clicked() {
                self.open_settings(SettingsTab::General);
            }
        });
    }

    fn avatar(&self, ui: &mut egui::Ui, size: f32) -> egui::Response {
        let p = Palette::of(ui);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
        let initial = self.settings.display_name.trim().chars().next().map(|c| c.to_uppercase().to_string());
        let fill = if resp.hovered() { p.muted } else { p.selected };
        ui.painter().circle_filled(rect.center(), size / 2.0, fill);
        match initial {
            Some(i) => {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, i, theme::font_bold(size * 0.45), p.text);
            }
            None => icons::paint(ui.painter(), rect.shrink(size * 0.24), Icon::Gear, p.text),
        }
        resp
    }

    fn sidebar_expanded(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);

        // Brand row.
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(132.0, 34.0), Sense::click());
            let logo = egui::Rect::from_min_size(rect.min + vec2(0.0, 3.0), Vec2::splat(28.0));
            icons::paint_logo(ui.painter(), logo, p.text, p.sidebar);
            ui.painter().text(pos2(logo.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, "Free Bot", theme::font_bold(18.0), p.text);
            if resp.on_hover_text("New chat").clicked() {
                self.new_chat();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Sidebar, "Close sidebar").clicked() {
                    self.settings.sidebar_open = false;
                    self.mark_settings_dirty();
                }
            });
        });
        ui.add_space(12.0);

        if widgets::nav_row(ui, Icon::Search, "Search", &shortcut_text(ui.ctx(), &SC_SEARCH)).clicked() {
            self.open_search();
        }
        if widgets::nav_row(ui, Icon::Compose, "New chat", &shortcut_text(ui.ctx(), &SC_NEW_CHAT)).clicked() {
            self.new_chat();
        }
        ui.add_space(10.0);

        // Profile row pinned to the bottom; history fills the space between.
        let bottom_h = 52.0;
        let avail = ui.available_height() - bottom_h;
        ui.allocate_ui(vec2(ui.available_width(), avail.max(0.0)), |ui| {
            egui::ScrollArea::vertical().id_salt("history").auto_shrink([false, false]).show(ui, |ui| {
                self.history_list(ui);
            });
        });

        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, p.border));
        ui.add_space(8.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
        }
        let name = if self.settings.display_name.trim().is_empty() {
            "Settings".to_string()
        } else {
            self.settings.display_name.trim().to_string()
        };
        let mut child = ui.new_child(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 5.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        self.avatar(&mut child, 30.0);
        child.add_space(4.0);
        child.label(egui::RichText::new(name).font(theme::font(14.5)).color(p.text));
        icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(18.0)),
            Icon::Gear,
            p.muted,
        );
        if resp.on_hover_text("Settings").clicked() {
            self.open_settings(SettingsTab::General);
        }
    }

    fn history_list(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        if self.convs.is_empty() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Your chats will show up here.").font(theme::font(13.0)).color(p.faint));
            });
            return;
        }
        let today = chrono::Local::now().date_naive();
        let mut items: Vec<(DateGroup, String, String)> =
            self.convs.iter().map(|c| (DateGroup::of(c.updated, c.pinned, today), c.id.clone(), c.title.clone())).collect();
        items.sort_by_key(|(g, _, _)| *g);

        let mut last_group = None;
        for (group, id, title) in items {
            if last_group != Some(group) {
                last_group = Some(group);
                ui.add_space(if group == DateGroup::Pinned { 2.0 } else { 10.0 });
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(group.label()).font(theme::font(12.5)).color(p.faint));
                });
                ui.add_space(2.0);
            }
            self.history_row(ui, &id, &title);
        }
        ui.add_space(8.0);
    }

    fn history_row(&mut self, ui: &mut egui::Ui, id: &str, title: &str) {
        let p = Palette::of(ui);
        let selected = self.current.as_deref() == Some(id);

        if let Some((rid, text)) = &mut self.renaming
            && rid == id
        {
            let mut done = None;
            let focus = std::mem::take(&mut self.rename_focus);
            widgets::input_frame(ui).show(ui, |ui| {
                let r = ui
                    .add(egui::TextEdit::singleline(text).frame(egui::Frame::NONE).margin(egui::Margin::ZERO).desired_width(f32::INFINITY));
                if focus {
                    r.request_focus();
                }
                if r.lost_focus() {
                    done = Some(!ui.input(|i| i.key_pressed(egui::Key::Escape)));
                }
            });
            if let Some(save) = done {
                let (rid, text) = self.renaming.take().unwrap();
                if save {
                    self.rename_conv(&rid, text);
                }
            }
            return;
        }

        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        let menu_id = ui.scope_id().with(("conv-menu", id));
        let menu_open = egui::Popup::is_id_open(ui.ctx(), menu_id);
        let hovered = resp.hovered() || menu_open || ui.rect_contains_pointer(rect);
        if selected {
            ui.painter().rect_filled(rect, CornerRadius::same(10), p.selected);
        } else if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(10), p.hover);
        }
        let streaming = self.is_streaming(id);
        let right_pad = if hovered {
            36.0
        } else if streaming {
            24.0
        } else {
            10.0
        };
        let font = theme::font(14.0);
        let text = widgets::truncate_to_width(ui, title, &font, rect.width() - 10.0 - right_pad);
        ui.painter().text(
            pos2(rect.left() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            font,
            if selected { p.text } else { p.text.gamma_multiply(0.92) },
        );
        if streaming && !hovered {
            let t = ui.input(|i| i.time) as f32;
            let a = 0.4 + 0.6 * (t * 4.0).sin().abs();
            ui.painter().circle_filled(pos2(rect.right() - 14.0, rect.center().y), 3.5, p.text.gamma_multiply(a));
        }
        if resp.clicked() {
            self.open_conv(id.to_string());
        }
        // Double-click renames, like most chat apps.
        if resp.double_clicked() {
            self.start_rename(id, title);
        }

        if hovered {
            let btn = egui::Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(26.0));
            let bresp = ui.interact(btn, menu_id.with("btn"), Sense::click());
            if bresp.hovered() {
                ui.painter().rect_filled(btn, CornerRadius::same(7), p.surface_hover);
            }
            icons::paint(ui.painter(), btn.shrink(5.0), Icon::Dots, p.text);
            let pinned = self.conv(id).is_some_and(|c| c.pinned);
            let id_owned = id.to_string();
            let title_owned = title.to_string();
            egui::Popup::menu(&bresp).id(menu_id).width(180.0).show(|ui| {
                ui.set_min_width(170.0);
                if menu_item(ui, Icon::Pin, if pinned { "Unpin" } else { "Pin" }, false).clicked() {
                    self.toggle_pin(&id_owned);
                }
                if menu_item(ui, Icon::Pencil, "Rename", false).clicked() {
                    self.start_rename(&id_owned, &title_owned);
                }
                if menu_item(ui, Icon::Download, "Export as Markdown", false).clicked() {
                    self.export_conv_markdown(&id_owned);
                }
                ui.separator();
                if menu_item(ui, Icon::Trash, "Delete", true).clicked() {
                    self.confirm = Some(Confirm::DeleteChat(id_owned.clone()));
                }
            });
        }
    }
}

/// A row inside a dropdown menu.
pub fn menu_item(ui: &mut egui::Ui, icon: Icon, label: &str, danger: bool) -> egui::Response {
    let p = Palette::of(ui);
    let color = if danger { p.danger } else { p.text };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(160.0), 34.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
    }
    icons::paint(ui.painter(), egui::Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(18.0)), icon, color);
    ui.painter().text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, label, theme::font(14.0), color);
    resp
}
