//! The modal dialogs drawn over the main window, and the settings window.

use crate::api::Guard;
use crate::config::*;
use crate::flow::*;
use crate::i18n::{Lang, t};
use crate::theme::*;
use crate::widgets::*;
use crate::{App, tf};
use eframe::egui::{self, Align, Align2, FontId, Layout, RichText, Stroke, Vec2};

/// Tabs of the settings window. Everything used to sit in one column, which stopped
/// fitting the window; grouping reads better than a scrollbar.
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum SetTab {
    #[default]
    Vault,
    Window,
    Steam,
}

impl SetTab {
    pub(crate) const ALL: [SetTab; 3] = [SetTab::Vault, SetTab::Window, SetTab::Steam];

    pub(crate) fn title(self) -> &'static str {
        match self {
            SetTab::Vault => t("Vault", "Хранилище"),
            SetTab::Window => t("Window", "Окно"),
            SetTab::Steam => "Steam",
        }
    }
}

#[derive(Default)]
pub(crate) struct SettingsForm {
    pub(crate) pass1: String,
    pub(crate) pass2: String,
    pub(crate) error: Option<String>,
    pub(crate) tab: SetTab,
}

/// Actions that cannot run inside the modal's closure: they either open a native
/// dialog or reopen the vault underneath the UI that is currently being drawn.
#[derive(Default)]
struct SettingsAct {
    pub(crate) toast: Option<String>,
    pub(crate) close: bool,
    pub(crate) pick_dir: bool,
    pub(crate) reset_dir: bool,
}

impl App {
    // ---------------- Modal dialogs ----------------

    pub(crate) fn flow_modal(&mut self, ctx: &egui::Context) {
        if self.flow.is_none() {
            return;
        }
        let kind = self.flow.as_ref().unwrap().kind;
        if kind == FlowKind::Settings {
            self.settings_modal(ctx);
            return;
        }
        let modal = egui::Modal::new(egui::Id::unique("flow")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.set_max_width(400.0); // do not stretch to fit long lines
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            match kind {
                FlowKind::Relogin(_) => self.ui_login(ctx, ui, t("Account sign-in", "Вход в аккаунт")),
                FlowKind::Link => self.ui_login(ctx, ui, t("Adding a new account", "Привязка нового аккаунта")),
                FlowKind::Revoke(id) => self.ui_revoke(ctx, ui, id),
                FlowKind::Forget(id) => self.ui_forget(ui, id),
                FlowKind::Settings => {}
            }
        });
        // Clicking outside only dismisses the dialog while nothing is in flight.
        let busy = self.flow.as_ref().is_some_and(|f| f.busy());
        if modal.should_close() && !busy {
            self.close_flow();
        }
    }

    fn ui_login(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, title: &str) {
        let step = self.flow.as_ref().unwrap().step.clone();
        let is_link = matches!(self.flow.as_ref().unwrap().kind, FlowKind::Link);
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(18.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !matches!(step, Step::Busy(_)) && button(ui, "✕", Btn::Ghost, true).clicked() {
                    self.close_flow();
                }
            });
        });
        if self.flow.is_none() {
            return; // dismissed with the close button
        }
        ui.add_space(10.0);

        match &step {
            Step::Credentials => {
                let (mut user, mut pass, mut agree) = {
                    let f = self.flow.as_ref().unwrap();
                    (f.user.clone(), f.pass.clone(), f.agree)
                };
                let locked_user = matches!(self.flow.as_ref().unwrap().kind, FlowKind::Relogin(_)) && !user.is_empty();
                ui.label(RichText::new(t("Steam login", "Логин Steam")).size(12.5).color(MUTED));
                ui.add_enabled(
                    !locked_user,
                    egui::TextEdit::singleline(&mut user).desired_width(ui.available_width()).margin(Vec2::new(10.0, 7.0)),
                );
                ui.add_space(6.0);
                ui.label(RichText::new(t("Password", "Пароль")).size(12.5).color(MUTED));
                let pr = ui.add(
                    egui::TextEdit::singleline(&mut pass).password(true).desired_width(ui.available_width()).margin(Vec2::new(10.0, 7.0)),
                );
                let enter = pr.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                if is_link {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut agree, "");
                        ui.label(RichText::new(t("I understand this authenticator replaces the current Steam Guard method, and the account needs a phone number.", "Понимаю: аутентификатор заменит текущую защиту Steam Guard. К аккаунту должен быть привязан телефон.")).size(12.0).color(MUTED));
                    });
                    ui.add_space(6.0);
                }
                if let Some(e) = &self.flow.as_ref().unwrap().error {
                    banner(ui, RED, e);
                    ui.add_space(6.0);
                }
                {
                    let f = self.flow.as_mut().unwrap();
                    f.user = user;
                    f.pass = pass;
                    f.agree = agree;
                }
                let ready = {
                    let f = self.flow.as_ref().unwrap();
                    !f.user.trim().is_empty() && !f.pass.is_empty() && (!is_link || f.agree)
                };
                ui.horizontal(|ui| {
                    if button(ui, if is_link { t("Continue", "Продолжить") } else { t("Sign in", "Войти") }, Btn::Primary, ready).clicked()
                        || (enter && ready)
                    {
                        self.start_login(ctx);
                    }
                    ui.label(
                        RichText::new(t("Your data goes only to Steam's servers", "Данные уходят только на серверы Steam"))
                            .size(11.0)
                            .color(FAINT),
                    );
                });
            }
            Step::Busy(msg) | Step::Waiting(msg) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(msg).color(MUTED));
                });
                if matches!(step, Step::Waiting(_)) {
                    ui.add_space(8.0);
                    if button(ui, t("Cancel", "Отмена"), Btn::Plain, true).clicked() {
                        self.close_flow();
                    }
                }
            }
            Step::EmailCode(hint) => {
                let msg = if hint.is_empty() {
                    t("Enter the code from the Steam Guard app", "Введите код из приложения Steam Guard").to_string()
                } else {
                    tf!("Enter the code sent to {hint}", "Введите код, отправленный на {hint}")
                };
                ui.label(RichText::new(msg).color(MUTED));
                ui.add_space(8.0);
                let mut code = self.flow.as_ref().unwrap().code.clone();
                let r = ui.add(
                    egui::TextEdit::singleline(&mut code)
                        .hint_text(t("code", "код"))
                        .desired_width(140.0)
                        .font(FontId::monospace(20.0))
                        .margin(Vec2::new(10.0, 8.0)),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                self.flow.as_mut().unwrap().code = code.clone();
                ui.add_space(6.0);
                if let Some(e) = &self.flow.as_ref().unwrap().error {
                    banner(ui, RED, e);
                    ui.add_space(6.0);
                }
                // Work out which kind of code this is from the offered methods.
                let is_email =
                    self.flow.as_ref().unwrap().session.as_ref().is_some_and(|s| s.guards.iter().any(|(g, _)| *g == Guard::EmailCode));
                let kind = if is_email { Guard::EmailCode } else { Guard::DeviceCode };
                if button(ui, t("Submit", "Подтвердить"), Btn::Primary, code.trim().len() >= 5).clicked()
                    || (enter && code.trim().len() >= 5)
                {
                    self.submit_code(ctx, code.trim().to_string(), kind);
                }
            }
            Step::Done(msg) => {
                banner(ui, GREEN, msg);
                ui.add_space(8.0);
                if button(ui, t("Done", "Готово"), Btn::Primary, true).clicked() {
                    self.close_flow();
                }
            }
            Step::Activate => self.ui_activate(ctx, ui),
            Step::Confirm => {}
        }
    }

    fn ui_activate(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let f = self.flow.as_ref().unwrap();
        let (revocation, confirm_type, hint) = f.linking.as_ref().map(|(_, _, r, t, h)| (r.clone(), *t, h.clone())).unwrap_or_default();
        banner(
            ui,
            YELLOW,
            &tf!(
                "IMPORTANT: save the revocation code {revocation}. Without it the authenticator cannot be removed.",
                "ВАЖНО: сохраните код восстановления {revocation} — без него нельзя снять аутентификатор."
            ),
        );
        ui.add_space(8.0);
        if button(ui, t("Copy revocation code", "Скопировать код восстановления"), Btn::Plain, true).clicked() {
            ui.ctx().copy_text(revocation.clone());
            self.toast(t("Revocation code copied", "Код восстановления скопирован"), false);
        }
        ui.add_space(10.0);
        let where_from = if confirm_type == 3 {
            tf!("to your email{}", "на почту{}", if hint.is_empty() { String::new() } else { format!(" ({hint})") })
        } else {
            tf!("by SMS{}", "по SMS{}", if hint.is_empty() { String::new() } else { tf!(" to {hint}", " на {hint}") })
        };
        ui.label(RichText::new(tf!("Enter the code sent {where_from}:", "Введите код, отправленный {where_from}:")).color(MUTED));
        ui.add_space(6.0);
        let mut code = self.flow.as_ref().unwrap().code.clone();
        let r = ui.add(
            egui::TextEdit::singleline(&mut code)
                .hint_text(t("activation code", "код активации"))
                .desired_width(180.0)
                .font(FontId::monospace(18.0))
                .margin(Vec2::new(10.0, 8.0)),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        self.flow.as_mut().unwrap().code = code.clone();
        ui.add_space(6.0);
        if let Some(e) = &self.flow.as_ref().unwrap().error {
            banner(ui, RED, e);
            ui.add_space(6.0);
        }
        if button(ui, t("Activate", "Активировать"), Btn::Primary, !code.trim().is_empty()).clicked() || (enter && !code.trim().is_empty())
        {
            self.start_finalize(ctx);
        }
    }

    fn ui_revoke(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, id: u64) {
        let step = self.flow.as_ref().unwrap().step.clone();
        let name = self.account(id).map(|a| self.display_name(a)).unwrap_or_default();
        ui.label(RichText::new(t("Remove authenticator", "Отвязать аутентификатор")).size(18.0).strong());
        ui.add_space(8.0);
        if let Step::Busy(msg) = &step {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(msg).color(MUTED));
            });
            return;
        }
        banner(
            ui,
            YELLOW,
            &tf!(
                "Steam Guard for \"{name}\" will fall back to email codes. The maFile leaves SDA, with a backup kept.",
                "Steam Guard для «{name}» вернётся к кодам на почту. maFile будет убран из SDA (с резервной копией)."
            ),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(t("Revocation code (Rxxxxx):", "Код восстановления (Rxxxxx):")).size(12.5).color(MUTED));
        let mut code = self.flow.as_ref().unwrap().code.clone();
        ui.add(
            egui::TextEdit::singleline(&mut code)
                .desired_width(ui.available_width())
                .font(FontId::monospace(16.0))
                .margin(Vec2::new(10.0, 8.0)),
        );
        self.flow.as_mut().unwrap().code = code.clone();
        ui.add_space(6.0);
        if let Some(e) = &self.flow.as_ref().unwrap().error {
            banner(ui, RED, e);
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            if button(ui, t("Remove", "Отвязать"), Btn::Danger, code.trim().len() > 1).clicked() {
                self.start_revoke(ctx, id);
            }
            if button(ui, t("Cancel", "Отмена"), Btn::Plain, true).clicked() {
                self.close_flow();
            }
        });
    }

    fn ui_forget(&mut self, ui: &mut egui::Ui, id: u64) {
        let name = self.account(id).map(|a| self.display_name(a)).unwrap_or_default();
        ui.label(RichText::new(t("Remove from SDA", "Убрать из SDA")).size(18.0).strong());
        ui.add_space(8.0);
        banner(
            ui,
            RED,
            t(
                "The authenticator stays attached in Steam. Without this maFile, or the revocation code, you will not get codes. Make sure you have a backup.",
                "Аутентификатор в Steam останется привязан! Без этого maFile (или кода восстановления) вы не получите коды. Убедитесь, что есть резервная копия.",
            ),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(tf!("Delete the account file for \"{name}\"?", "Удалить файл аккаунта «{name}»?")).color(MUTED));
        ui.add_space(10.0);
        let mut remove = false;
        ui.horizontal(|ui| {
            if button(ui, t("Yes, remove the file", "Да, убрать файл"), Btn::Danger, true).clicked() {
                remove = true;
            }
            if button(ui, t("Cancel", "Отмена"), Btn::Plain, true).clicked() {
                self.close_flow();
            }
        });
        if remove {
            if let Some(acc) = self.account(id).cloned()
                && let Some(store) = &mut self.store
            {
                match store.remove(&acc) {
                    Ok(()) => self.toast(tf!("{name} removed from SDA", "{name} убран из SDA"), false),
                    Err(e) => self.toast(e, true),
                }
            }
            self.close_flow();
            self.selected = None;
            self.reload_accounts();
        }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        let mut act = SettingsAct::default();
        let modal = egui::Modal::new(egui::Id::unique("settings")).show(ctx, |ui| {
            ui.set_width(392.0);
            ui.set_max_width(392.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(RichText::new(t("Settings", "Настройки")).size(18.0).strong());
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for tab in SetTab::ALL {
                    let kind = if self.settings.tab == tab { Btn::Plain } else { Btn::Ghost };
                    if button(ui, tab.title(), kind, true).clicked() {
                        self.settings.tab = tab;
                    }
                }
            });
            ui.add_space(12.0);

            // Tabs differ in height, so pad to a shared one; otherwise the modal
            // would jump every time you switch.
            let top = ui.cursor().top();
            match self.settings.tab {
                SetTab::Vault => self.tab_vault(ui, &mut act),
                SetTab::Window => self.tab_window(ui),
                SetTab::Steam => self.tab_steam(ui, ctx),
            }
            ui.add_space((262.0 - (ui.cursor().top() - top)).max(12.0));

            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if button(ui, t("Done", "Готово"), Btn::Primary, true).clicked() {
                    act.close = true;
                }
            });
        });
        if let Some(t) = act.toast {
            self.toast(t, false);
        }
        if act.pick_dir {
            self.choose_dir(ctx);
            return;
        }
        if act.reset_dir {
            self.switch_dir(ctx, default_mafiles_dir());
            return;
        }
        if act.close || modal.should_close() {
            self.close_flow();
        }
    }

    /// The Vault tab: maFiles encryption and the directory they are read from.
    fn tab_vault(&mut self, ui: &mut egui::Ui, act: &mut SettingsAct) {
        ui.label(RichText::new(t("maFiles encryption", "Шифрование maFiles")).strong());
        let encrypted = self.store.as_ref().is_some_and(|s| s.encrypted());
        ui.label(
            RichText::new(if encrypted {
                t("On", "Включено")
            } else {
                t("Off — the files are stored in plain text", "Выключено — файлы лежат открытым текстом")
            })
            .color(if encrypted { GREEN } else { YELLOW }),
        );
        let w = ui.available_width();
        input(ui, &mut self.settings.pass1, if encrypted { t("New password", "Новый пароль") } else { t("Password", "Пароль") }, true, w);
        input(ui, &mut self.settings.pass2, t("Repeat the password", "Повторите пароль"), true, w);
        if let Some(e) = &self.settings.error {
            ui.label(RichText::new(e).color(RED));
        }
        let mut change: Option<Option<String>> = None;
        ui.horizontal(|ui| {
            let ok = !self.settings.pass1.is_empty();
            if button(ui, if encrypted { t("Change password", "Сменить пароль") } else { t("Encrypt", "Зашифровать") }, Btn::Primary, ok)
                .clicked()
            {
                if self.settings.pass1 != self.settings.pass2 {
                    self.settings.error = Some(t("Passwords do not match", "Пароли не совпадают").into());
                } else {
                    change = Some(Some(self.settings.pass1.clone()));
                }
            }
            if encrypted && button(ui, t("Remove encryption", "Снять шифрование"), Btn::Danger, true).clicked() {
                change = Some(None);
            }
        });
        if let Some(new) = change {
            self.flush_meta();
            let accounts = self.accounts.clone();
            if let Some(store) = &mut self.store {
                match store.set_passkey(&accounts, new.as_deref()) {
                    Ok(()) => {
                        self.settings.pass1.clear();
                        self.settings.pass2.clear();
                        self.settings.error = None;
                        act.toast = Some(if new.is_some() {
                            t("Encryption password set", "Пароль шифрования установлен").into()
                        } else {
                            t("Encryption removed", "Шифрование снято").into()
                        });
                    }
                    Err(e) => self.settings.error = Some(e),
                }
            }
        }

        ui.add_space(14.0);
        ui.label(RichText::new(t("maFiles folder", "Папка maFiles")).strong());
        let dir = self.current_dir();
        let is_default = dir == default_mafiles_dir();
        ui.label(RichText::new(dir.display().to_string()).size(12.0).color(FAINT));
        if is_default {
            ui.label(RichText::new(t("default folder", "стандартная папка")).size(11.0).color(FAINT));
        }
        let mut reread = false;
        ui.horizontal(|ui| {
            if button(ui, t("Choose folder…", "Выбрать папку…"), Btn::Plain, true).clicked() {
                act.pick_dir = true;
            }
            if button(ui, t("Open", "Открыть"), Btn::Plain, true).clicked() {
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
            if button(ui, t("Reload", "Перечитать"), Btn::Ghost, true).clicked() {
                reread = true;
            }
        });
        if !is_default && button(ui, t("Back to default", "Вернуть стандартную"), Btn::Ghost, true).clicked() {
            act.reset_dir = true;
        }
        if reread {
            self.reload_accounts();
        }
    }

    /// The Window tab: interface language, backdrop, and what minimise does.
    fn tab_window(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(t("Language", "Язык")).strong());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for lang in Lang::ALL {
                let kind = if crate::i18n::get() == lang { Btn::Plain } else { Btn::Ghost };
                if button(ui, lang.label(), kind, true).clicked() {
                    crate::i18n::set(lang);
                    let mut cfg = load_config();
                    cfg.lang = Some(lang.code().to_string());
                    let _ = save_config(&cfg);
                }
            }
        });

        ui.add_space(14.0);
        ui.label(RichText::new(t("Backdrop", "Фон")).strong());
        let mut blur = self.blur;
        if ui.checkbox(&mut blur, t("Blur the window backdrop", "Размытие фона окна")).changed() {
            self.blur = blur;
            let mut cfg = load_config();
            cfg.blur = blur;
            let _ = save_config(&cfg);
        }
        ui.label(
            RichText::new(t(
                "Experimental: the system acrylic backdrop, showing through the window background \
                 and the sidebar. It can stutter on weak machines and in remote sessions.",
                "Экспериментально: системный acrylic — размытие идёт сквозь фон окна и сайдбар. \
                 На слабых машинах и в удалённых сессиях может подтормаживать.",
            ))
            .size(11.5)
            .color(FAINT),
        );

        ui.add_space(14.0);
        ui.label(RichText::new(t("Tray", "Трей")).strong());
        let mut to_tray = self.minimize_to_tray;
        if ui.checkbox(&mut to_tray, t("Minimise to tray", "Сворачивать в трей")).changed() {
            self.minimize_to_tray = to_tray;
            let mut cfg = load_config();
            cfg.minimize_to_tray = to_tray;
            let _ = save_config(&cfg);
        }
        ui.label(
            RichText::new(if self.tray.is_some() {
                t(
                    "Minimise sends the window to the tray. Left-click the icon to bring it back, \
                     right-click for the account list, where a click copies the code.",
                    "Кнопка «—» убирает окно в трей. ЛКМ по иконке — вернуть окно, \
                     ПКМ — список аккаунтов: клик копирует код.",
                )
            } else {
                t(
                    "The tray icon could not be created, so minimise behaves normally.",
                    "Иконку в трее создать не удалось — «—» сворачивает окно обычным образом.",
                )
            })
            .size(11.5)
            .color(FAINT),
        );
    }

    /// The Steam tab: what to pull from the profile, and clock sync.
    fn tab_steam(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new(t("Account data", "Данные аккаунтов")).strong());
        let mut enrich = self.enrich;
        if ui.checkbox(&mut enrich, t("Fetch persona name and avatar", "Получать ник и аватар")).changed() {
            self.enrich = enrich;
            let mut cfg = load_config();
            cfg.enrich = enrich;
            let _ = save_config(&cfg);
        }
        ui.label(
            RichText::new(t(
                "Read from the public Steam profile at sign-in and at startup. No Web API key needed.",
                "Берётся из публичного профиля Steam при входе и на старте. Без Web API ключа.",
            ))
            .size(11.5)
            .color(FAINT),
        );

        ui.add_space(14.0);
        ui.label(RichText::new(t("Steam time", "Время Steam")).strong());
        let mut sync = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tf!("Offset: {:+} s", "Смещение: {:+} с", self.time_offset)).color(MUTED));
            if button(
                ui,
                if self.syncing { t("Syncing…", "Синхронизация…") } else { t("Sync", "Синхронизировать") },
                Btn::Plain,
                !self.syncing,
            )
            .clicked()
            {
                sync = true;
            }
        });
        if sync {
            self.sync_time(ctx);
        }
        ui.label(
            RichText::new(t(
                "Codes are derived from Steam's server time. Sync if the PC clock has drifted.",
                "Коды считаются от времени серверов Steam. Сверка нужна, если часы на ПК врут.",
            ))
            .size(11.5)
            .color(FAINT),
        );
    }

    pub(crate) fn draw_toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        for t in &mut self.toasts {
            if t.born.is_nan() {
                t.born = now;
            }
        }
        self.toasts.retain(|t| now - t.born < 4.0);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-18.0, -18.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in &self.toasts {
                    let fade = ((4.0 - (now - t.born)) as f32 * 2.0).clamp(0.0, 1.0);
                    let col = if t.error { RED } else { GREEN };
                    egui::Frame::new()
                        .fill(SURFACE.gamma_multiply(fade))
                        .stroke(Stroke::new(1.0, col.gamma_multiply(0.5 * fade)))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_max_width(340.0);
                            ui.label(RichText::new(&t.text).color(mix(TEXT, col, 0.25).gamma_multiply(fade)));
                        });
                    ui.add_space(6.0);
                }
            });
        ctx.request_repaint();
    }
}
