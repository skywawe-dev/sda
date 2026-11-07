//! The main window: title bar, sidebar and the content cards.

use crate::flow::*;
use crate::i18n::t;
use crate::theme::*;
use crate::widgets::*;
use crate::{App, Screen, tf};
use eframe::egui::{self, Align, Align2, CornerRadius, FontId, Layout, RichText, Sense, Stroke, Vec2};
use serde_json::Value;
impl App {
    pub(crate) fn title_bar(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click_and_drag());
        if resp.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let p = ui.painter();
        let lc = rect.left_center() + Vec2::new(26.0, 0.0);
        let shield = vec![
            lc + Vec2::new(-8.0, -9.0),
            lc + Vec2::new(8.0, -9.0),
            lc + Vec2::new(8.0, 1.0),
            lc + Vec2::new(0.0, 10.0),
            lc + Vec2::new(-8.0, 1.0),
        ];
        p.add(egui::Shape::convex_polygon(shield, ACCENT.gamma_multiply(0.25), Stroke::new(1.5, ACCENT)));
        let t = p.text(lc + Vec2::new(18.0, 0.0), Align2::LEFT_CENTER, "Steam Desktop Authenticator", FontId::proportional(15.0), TEXT);
        p.text(t.right_center() + Vec2::new(8.0, 1.0), Align2::LEFT_CENTER, "rust", FontId::proportional(12.0), FAINT);
        p.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, BORDER));

        let mut x = rect.right() - 22.0;
        for (icon, close) in [("✕", true), ("—", false)] {
            let r = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(28.0));
            let resp = ui.interact(r, ui.scope_id().with(icon), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 6, if close { RED.gamma_multiply(0.2) } else { HOVER });
            }
            let col = if resp.hovered() && close {
                RED
            } else if resp.hovered() {
                TEXT
            } else {
                MUTED
            };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(14.0), col);
            if resp.clicked() {
                if close {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                } else if self.minimize_to_tray && self.tray.is_some() && self.hwnd != 0 {
                    crate::tray::hide_window(self.hwnd);
                } else {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
            x -= 34.0;
        }
    }

    pub(crate) fn unlock_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(110.0);
            ui.label(RichText::new(t("The vault is encrypted", "Хранилище зашифровано")).size(21.0).strong());
            ui.label(RichText::new(t("Enter the maFiles encryption password", "Введите пароль шифрования maFiles")).color(MUTED));
            ui.add_space(14.0);
            let r = input(ui, &mut self.passkey_input, t("Password", "Пароль"), true, 280.0);
            if !r.has_focus() && self.passkey_input.is_empty() {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(6.0);
            if let Some(e) = &self.unlock_error {
                ui.label(RichText::new(e).color(RED));
            }
            ui.add_space(6.0);
            if button(ui, t("Unlock", "Разблокировать"), Btn::Primary, !self.passkey_input.is_empty()).clicked() || enter {
                let pass = self.passkey_input.clone();
                if let Some(store) = &mut self.store {
                    match store.unlock(&pass) {
                        Ok(()) => {
                            self.passkey_input.clear();
                            self.unlock_error = None;
                            self.screen = Screen::Main;
                            self.reload_accounts();
                            if let Some(id) = self.selected {
                                let ctx = ui.ctx().clone();
                                self.load_confs(&ctx, id);
                            }
                        }
                        Err(e) => self.unlock_error = Some(e),
                    }
                }
            }
        });
    }

    pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let size = Vec2::new(262.0, ui.available_height());
        ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
            ui.set_min_size(size);
            let full = ui.max_rect();
            // With the backdrop on, the sidebar gets no fill at all. The window
            // background beneath it is already translucent, and a second layer would
            // only dim the acrylic: the blur must read the same here as everywhere.
            if !self.blur {
                ui.painter().rect_filled(full, CornerRadius { nw: 0, ne: 0, sw: 12, se: 0 }, SIDEBAR);
            }
            ui.painter().vline(full.right(), full.y_range(), Stroke::new(1.0, BORDER));

            egui::Frame::new().inner_margin(12).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                input(ui, &mut self.search, t("Search", "Поиск"), false, ui.available_width());
                ui.add_space(4.0);
                ui.label(RichText::new(tf!("ACCOUNTS · {}", "АККАУНТЫ · {}", self.accounts.len())).size(11.0).color(FAINT));

                let bottom_h = 84.0;
                let list_h = (ui.available_height() - bottom_h).max(60.0);
                let ctx = ui.ctx().clone();
                let needle = self.search.to_lowercase();

                // Row data is collected up front so the loop holds no borrow of self.
                let rows: Vec<(u64, String, String, bool)> = self
                    .accounts
                    .iter()
                    .filter_map(|acc| {
                        let alias = self.alias(acc.steamid).to_string();
                        let persona = self.persona(acc.steamid).to_string();
                        let login = acc.name().to_string();
                        if !needle.is_empty()
                            && !alias.to_lowercase().contains(&needle)
                            && !persona.to_lowercase().contains(&needle)
                            && !login.to_lowercase().contains(&needle)
                            && !acc.steamid.to_string().contains(&needle)
                        {
                            return None;
                        }
                        // Name preference: alias, then persona, then login.
                        let primary = if !alias.is_empty() {
                            alias
                        } else if !persona.is_empty() {
                            persona
                        } else {
                            login.clone()
                        };
                        let secondary = if primary == login { format!("ID {}", acc.steamid) } else { login };
                        let enrolled = acc.raw.get("fully_enrolled").and_then(Value::as_bool).unwrap_or(true);
                        Some((acc.steamid, primary, secondary, enrolled))
                    })
                    .collect();

                let mut clicked = None;
                let mut new_order: Option<Vec<u64>> = None;
                // Reordering only makes sense unfiltered, and with more than one account.
                let reorderable = needle.is_empty() && rows.len() > 1;
                let drag_id = self.dragging.filter(|_| reorderable);
                egui::ScrollArea::vertical().id_salt("accounts").max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                    if self.accounts.is_empty() {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(t(
                                "Nothing here yet.\nAdd an account with the button below.",
                                "Пока пусто.\nДобавьте аккаунт кнопкой ниже.",
                            ))
                            .color(MUTED),
                        );
                        return;
                    } else if rows.is_empty() {
                        ui.add_space(16.0);
                        ui.label(RichText::new(t("Nothing found", "Ничего не найдено")).color(MUTED));
                        return;
                    }
                    let pointer_y = ui.input(|i| i.pointer.interact_pos()).map(|p| p.y);
                    let mut rects: Vec<(u64, egui::Rect)> = Vec::with_capacity(rows.len());
                    let mut started: Option<u64> = None;
                    let mut stopped = false;

                    for (steamid, primary, secondary, enrolled) in &rows {
                        let sense = if reorderable { Sense::click_and_drag() } else { Sense::click() };
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), sense);
                        rects.push((*steamid, rect));
                        let is_drag = drag_id == Some(*steamid);
                        let t = ctx.animate_bool(resp.id, self.selected == Some(*steamid));
                        let h = ctx.animate_bool(resp.id.with("h"), resp.hovered() && drag_id.is_none());
                        let badge = self.confs.get(steamid).filter(|s| s.loaded).map(|s| s.list.len());
                        // The dragged row leaves a faint gap; the row itself is drawn
                        // later as a ghost following the cursor.
                        let alpha = if is_drag { 0.25 } else { 1.0 };
                        let tex = self.enrich.then(|| self.avatars.get(steamid)).flatten();
                        draw_account_row(ui, rect, primary, secondary, *enrolled, badge, tex, t, h, alpha);
                        if reorderable {
                            if resp.drag_started() {
                                started = Some(*steamid);
                            }
                            if resp.drag_stopped() && is_drag {
                                stopped = true;
                            }
                            let cursor = if is_drag { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab };
                            resp.clone().on_hover_cursor(cursor);
                        }
                        if resp.clicked() {
                            clicked = Some(*steamid);
                        }
                    }
                    if let Some(id) = started {
                        self.dragging = Some(id);
                    }

                    // The insertion marker, then the ghost of the dragged row.
                    if let (Some(did), Some(py)) = (drag_id, pointer_y) {
                        let others: Vec<(u64, egui::Rect)> = rects.iter().copied().filter(|(id, _)| *id != did).collect();
                        let target = others.iter().filter(|(_, r)| r.center().y < py).count();
                        if let Some((_, first)) = others.first() {
                            let y = if target == 0 { first.top() - 1.0 } else { others[target - 1].1.bottom() + 1.0 };
                            let x = first.x_range();
                            ui.painter().hline(x, y, Stroke::new(2.0, ACCENT));
                        }
                        if let Some((_, _, p2, s2, e2)) =
                            rows.iter().enumerate().find(|(_, r)| r.0 == did).map(|(i, r)| (i, r.0, &r.1, &r.2, r.3))
                        {
                            let w = rects.first().map(|(_, r)| r.width()).unwrap_or(240.0);
                            let ghost = egui::Rect::from_center_size(egui::pos2(rects[0].1.center().x, py), Vec2::new(w, 50.0));
                            ui.painter().rect_filled(ghost.expand(1.0), 8, SURFACE);
                            ui.painter().rect_stroke(ghost, 8, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                            let badge = self.confs.get(&did).filter(|s| s.loaded).map(|s| s.list.len());
                            let tex = self.enrich.then(|| self.avatars.get(&did)).flatten();
                            draw_account_row(ui, ghost, p2, s2, e2, badge, tex, 0.0, 0.6, 1.0);
                        }
                        if stopped {
                            let mut order: Vec<u64> = others.iter().map(|(id, _)| *id).collect();
                            order.insert(target.min(order.len()), did);
                            new_order = Some(order);
                        }
                    }
                    if stopped {
                        self.dragging = None;
                    }
                    if drag_id.is_some() {
                        ctx.request_repaint();
                    }
                });
                if let Some(order) = new_order {
                    self.apply_order(&order);
                }
                if let Some(id) = clicked {
                    self.select(&ctx, id);
                }

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if button(ui, t("Settings", "Настройки"), Btn::Ghost, true).clicked() {
                            self.open_flow(FlowKind::Settings);
                        }
                        let off = self.time_offset;
                        let label = if self.syncing {
                            t("syncing…", "синхр…").to_string()
                        } else {
                            tf!("Δt {off:+} s", "Δt {off:+} с")
                        };
                        ui.label(RichText::new(label).size(11.0).color(FAINT))
                            .on_hover_text(t("Difference from Steam's server time", "Разница с временем серверов Steam"));
                    });
                    if button(ui, t("+  Add account", "+  Привязать аккаунт"), Btn::Primary, true).clicked() {
                        self.open_flow(FlowKind::Link);
                    }
                    if button(ui, t("Import maFile", "Импорт maFile"), Btn::Plain, true).clicked() {
                        self.import(&ctx);
                    }
                });
            });
        });
    }

    pub(crate) fn content(&mut self, ui: &mut egui::Ui) {
        let size = ui.available_size();
        ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
            egui::Frame::new().inner_margin(18).show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if self.current().is_none() {
                    self.empty_state(ui);
                    return;
                }
                egui::ScrollArea::vertical().id_salt("content").auto_shrink([false, false]).show(ui, |ui| {
                    self.code_card(ui);
                    ui.add_space(12.0);
                    self.confirmations_card(ui);
                    ui.add_space(12.0);
                    self.notes_card(ui);
                    ui.add_space(12.0);
                    self.info_card(ui);
                });
            });
        });
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.label(RichText::new(t("No account selected", "Нет выбранного аккаунта")).size(21.0).strong());
            ui.label(
                RichText::new(t("Add a new account or import a maFile.", "Привяжите новый аккаунт или импортируйте maFile.")).color(MUTED),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 320.0).max(0.0) / 2.0);
                if button(ui, t("+  Add account", "+  Привязать аккаунт"), Btn::Primary, true).clicked() {
                    self.open_flow(FlowKind::Link);
                }
                if button(ui, t("Import maFile", "Импорт maFile"), Btn::Plain, true).clicked() {
                    let ctx = ui.ctx().clone();
                    self.import(&ctx);
                }
            });
        });
    }

    fn code_card(&mut self, ui: &mut egui::Ui) {
        let now = self.steam_time();
        let Some(acc) = self.current().cloned() else { return };
        let steamid = acc.steamid;
        let enrolled = acc.raw.get("fully_enrolled").and_then(Value::as_bool).unwrap_or(true);
        let code = crate::steam::generate_code(acc.field("shared_secret"), now).unwrap_or_else(|| "?????".into());
        let left = 30 - now.rem_euclid(30);
        let primary = self.display_name(&acc);
        let login = acc.name().to_string();
        let since_copy = ui.input(|i| i.time) - self.copied_at;

        let mut alias_val = self.alias(steamid).to_string();
        let mut alias_changed = false;
        let mut menu_action: Option<FlowKind> = None;

        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                let tex = self.enrich.then(|| self.avatars.get(&steamid)).flatten();
                avatar(ui, r.center(), 22.0, &primary, tex, 1.0);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    if self.editing_alias {
                        let r = input(ui, &mut alias_val, &login, false, 220.0);
                        alias_changed = r.changed();
                        if r.lost_focus() {
                            self.editing_alias = false;
                        } else {
                            r.request_focus();
                        }
                    } else {
                        // Clicking the name edits the alias; no pencil icon needed.
                        let name = ui.add(egui::Label::new(RichText::new(&primary).size(18.0).strong()).sense(Sense::click()));
                        if name
                            .on_hover_cursor(egui::CursorIcon::Text)
                            .on_hover_text(t("Click to set an alias", "Нажмите, чтобы задать алиас"))
                            .clicked()
                        {
                            self.editing_alias = true;
                        }
                    }
                    let sub = if primary == login { format!("SteamID {steamid}") } else { login.clone() };
                    ui.label(RichText::new(sub).size(12.0).color(FAINT));
                });
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    let resp = button(ui, "···", Btn::Plain, true);
                    egui::Popup::menu(&resp).show(|ui| {
                        ui.set_min_width(190.0);
                        if ui.add(egui::Button::new(t("Sign in again", "Войти заново")).frame(false)).clicked() {
                            menu_action = Some(FlowKind::Relogin(steamid));
                            ui.close();
                        }
                        if ui
                            .add(
                                egui::Button::new(RichText::new(t("Remove authenticator", "Отвязать аутентификатор")).color(YELLOW))
                                    .frame(false),
                            )
                            .clicked()
                        {
                            menu_action = Some(FlowKind::Revoke(steamid));
                            ui.close();
                        }
                        ui.separator();
                        if ui.add(egui::Button::new(RichText::new(t("Remove from SDA", "Убрать из SDA")).color(RED)).frame(false)).clicked()
                        {
                            menu_action = Some(FlowKind::Forget(steamid));
                            ui.close();
                        }
                    });
                });
            });

            if !enrolled {
                ui.add_space(8.0);
                banner(
                    ui,
                    YELLOW,
                    t(
                        "This authenticator is not active yet. Finish setup from the \u{00b7}\u{00b7}\u{00b7} menu, via Sign in again.",
                        "Аутентификатор ещё не активирован — завершите привязку через меню «···» → «Войти заново» или заново.",
                    ),
                );
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let font = FontId::monospace(40.0);
                let cell = 38.0;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(cell * 5.0 + 8.0, 64.0), Sense::click());
                let h = ui.ctx().animate_bool(resp.id, resp.hovered());
                ui.painter().rect_filled(rect, 12, mix(FIELD, HOVER, h));
                for (i, ch) in code.chars().enumerate() {
                    let c = egui::pos2(rect.left() + 4.0 + cell * (i as f32 + 0.5), rect.center().y);
                    ui.painter().text(c, Align2::CENTER_CENTER, ch, font.clone(), TEXT);
                }
                let copy =
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Click to copy", "Клик — скопировать")).clicked();
                ui.add_space(12.0);
                timer_ring(ui, left as f32 / 30.0, left);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(14.0);
                    let label = if since_copy < 1.5 { t("Copied", "Скопировано") } else { t("Copy", "Копировать") };
                    let kind = if since_copy < 1.5 { Btn::Success } else { Btn::Plain };
                    if button(ui, label, kind, true).clicked() || copy {
                        ui.ctx().copy_text(code.clone());
                        self.copied_at = ui.input(|i| i.time);
                    }
                });
            });
        });

        if alias_changed {
            let m = self.meta.entry(steamid).or_default();
            m.alias = alias_val.trim().to_string();
            self.meta_dirty = Some(ui.input(|i| i.time));
            self.sort_accounts();
        }
        if let Some(kind) = menu_action {
            match kind {
                FlowKind::Forget(id) => self.open_flow(FlowKind::Forget(id)),
                k => self.open_flow(k),
            }
        }
    }

    fn confirmations_card(&mut self, ui: &mut egui::Ui) {
        let Some(steamid) = self.selected else { return };
        let ctx = ui.ctx().clone();
        let mut action: Option<(Vec<(String, String)>, bool)> = None;
        let mut refresh = false;
        let state = self.confs.entry(steamid).or_default();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                section(ui, t("Confirmations", "Подтверждения"));
                if state.loaded && !state.list.is_empty() {
                    ui.label(RichText::new(format!(" {} ", state.list.len())).background_color(ACCENT.gamma_multiply(0.2)).color(ACCENT));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if state.loading { t("Loading…", "Загрузка…") } else { t("Refresh", "Обновить") };
                    if button(ui, label, Btn::Plain, !state.loading).clicked() {
                        refresh = true;
                    }
                    if state.list.len() > 1 {
                        let busy = !state.acting.is_empty();
                        if button(ui, t("Reject all", "Отклонить все"), Btn::Danger, !busy).clicked() {
                            action = Some((state.list.iter().map(|c| (c.id.clone(), c.nonce.clone())).collect(), false));
                        }
                        if button(ui, t("Accept all", "Принять все"), Btn::Success, !busy).clicked() {
                            action = Some((state.list.iter().map(|c| (c.id.clone(), c.nonce.clone())).collect(), true));
                        }
                    }
                });
            });
            ui.add_space(6.0);
            if let Some(e) = &state.error {
                banner(ui, RED, e);
            } else if !state.loaded {
                ui.label(RichText::new(t("Loading…", "Загружаем…")).color(MUTED));
            } else if state.list.is_empty() {
                ui.label(RichText::new(t("No pending confirmations", "Нет ожидающих подтверждений")).color(MUTED));
            }
            for c in &state.list {
                let busy = state.acting.contains(&c.id);
                egui::Frame::new().fill(FIELD).corner_radius(10).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let color = match c.kind {
                            2 => ACCENT,
                            3 => GREEN,
                            _ => YELLOW,
                        };
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(" {} ", c.kind_label()))
                                        .size(11.0)
                                        .color(color)
                                        .background_color(color.gamma_multiply(0.15)),
                                );
                                ui.label(RichText::new(ago(c.created)).size(11.0).color(FAINT));
                            });
                            ui.label(
                                RichText::new(if c.headline.is_empty() { t("Untitled", "Без названия") } else { &c.headline }).strong(),
                            );
                            for s in &c.summary {
                                ui.label(RichText::new(s).size(12.5).color(MUTED));
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if busy {
                                ui.spinner();
                                return;
                            }
                            if button(ui, t("Reject", "Отклонить"), Btn::Danger, true).clicked() {
                                action = Some((vec![(c.id.clone(), c.nonce.clone())], false));
                            }
                            if button(ui, t("Accept", "Принять"), Btn::Success, true).clicked() {
                                action = Some((vec![(c.id.clone(), c.nonce.clone())], true));
                            }
                        });
                    });
                });
            }
        });
        if refresh {
            self.load_confs(&ctx, steamid);
        }
        if let Some((items, accept)) = action {
            self.act(&ctx, steamid, items, accept);
        }
    }

    fn notes_card(&mut self, ui: &mut egui::Ui) {
        let Some(steamid) = self.selected else { return };
        let mut text = self.meta.get(&steamid).map(|m| m.note.clone()).unwrap_or_default();
        let mut changed = false;
        card(ui, |ui| {
            section(ui, t("Notes", "Заметки"));
            ui.add_space(6.0);
            let r = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .hint_text(
                        RichText::new(t("Private notes for this account, stored locally", "Личные заметки к аккаунту (хранятся локально)"))
                            .color(FAINT),
                    )
                    .desired_width(ui.available_width())
                    .desired_rows(3)
                    .margin(Vec2::new(10.0, 8.0)),
            );
            changed = r.changed();
        });
        if changed {
            self.meta.entry(steamid).or_default().note = text;
            self.meta_dirty = Some(ui.input(|i| i.time));
        }
    }

    fn info_card(&mut self, ui: &mut egui::Ui) {
        let Some(acc) = self.current().cloned() else { return };
        let show = self.show_revocation;
        let mut toggle = false;
        card(ui, |ui| {
            section(ui, t("Details", "Детали"));
            ui.add_space(6.0);
            fn row(ui: &mut egui::Ui, k: &str, add: impl FnOnce(&mut egui::Ui)) {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(160.0, 22.0), Sense::hover());
                    ui.painter().text(r.left_center(), Align2::LEFT_CENTER, k, FontId::proportional(13.5), MUTED);
                    add(ui);
                });
            }
            row(ui, t("Login", "Логин"), |ui| {
                ui.label(acc.name());
            });
            let rcode = acc.field("revocation_code").to_string();
            row(ui, t("Revocation code", "Код восстановления"), |ui| {
                ui.label(RichText::new(if show { rcode.as_str() } else { "R•••••" }).monospace());
                if button(ui, if show { t("Hide", "Скрыть") } else { t("Show", "Показать") }, Btn::Ghost, true).clicked() {
                    toggle = true;
                }
                if show && !rcode.is_empty() && button(ui, t("Copy", "Копировать"), Btn::Ghost, true).clicked() {
                    ui.ctx().copy_text(rcode.clone());
                }
            });
            let refresh_exp = crate::steam::jwt_exp(acc.session("RefreshToken"));
            row(ui, t("Session", "Сессия"), |ui| {
                let (text, color) = match refresh_exp {
                    None => (t("no data — sign in required", "нет данных — нужен вход").to_string(), YELLOW),
                    Some(e) if e < crate::steam::unix_now() => (t("expired — sign in again", "истекла — войдите заново").to_string(), RED),
                    Some(e) => (tf!("valid for {} more days", "действует ещё {} дн", (e - crate::steam::unix_now()) / 86400), GREEN),
                };
                ui.label(RichText::new(text).color(color));
            });
        });
        if toggle {
            self.show_revocation = !self.show_revocation;
        }
    }
}
