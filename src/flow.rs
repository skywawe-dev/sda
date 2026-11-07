//! The sign-in, enrolment and removal dialogs as a state machine.
//!
//! Every step that talks to Steam runs off-thread and reports back as a [`FlowEv`],
//! which [`App::on_flow`] folds into the current [`Flow`]. The generation counter
//! discards events from a dialog the user has already closed.

use crate::api::{ApiError, AuthSession, Guard, NewAuthenticator, Tokens};
use crate::i18n::t;
use crate::modals::SettingsForm;
use crate::{App, Msg, random_session_id, tf};
use eframe::egui::{self};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
pub(crate) enum FlowEv {
    Begun(Result<AuthSession, ApiError>),
    CodeSent(Result<(), ApiError>),
    LoggedIn(Result<Tokens, ApiError>),
    Added(Result<NewAuthenticator, ApiError>),
    Finalized(Result<(), ApiError>),
    Removed(Result<(), String>),
}

/// What the open dialog is for.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FlowKind {
    /// Sign in again to refresh an account's session.
    Relogin(u64),
    /// Add a new account.
    Link,
    /// Remove the authenticator using the revocation code.
    Revoke(u64),
    /// Drop the maFile from SDA.
    Forget(u64),
    Settings,
}

#[derive(Clone, PartialEq)]
pub(crate) enum Step {
    Credentials,
    Busy(String),
    /// Entering the code from the email.
    EmailCode(String),
    /// Waiting for confirmation in the app or by email.
    Waiting(String),
    /// Entering the SMS or email code that activates the authenticator.
    Activate,
    Confirm,
    Done(String),
}

pub(crate) struct Flow {
    pub(crate) kind: FlowKind,
    pub(crate) generation: u64,
    pub(crate) step: Step,
    pub(crate) user: String,
    pub(crate) pass: String,
    pub(crate) code: String,
    pub(crate) agree: bool,
    pub(crate) error: Option<String>,
    pub(crate) session: Option<AuthSession>,
    pub(crate) tokens: Option<Tokens>,
    /// For enrolment: steamid, shared_secret, revocation_code, confirm_type, phone_hint.
    pub(crate) linking: Option<(u64, String, String, i64, String)>,
    pub(crate) device_id: String,
    pub(crate) cancel: Arc<AtomicBool>,
}

impl Flow {
    pub(crate) fn new(kind: FlowKind, generation: u64) -> Flow {
        Flow {
            kind,
            generation,
            step: match kind {
                FlowKind::Relogin(_) | FlowKind::Link => Step::Credentials,
                _ => Step::Confirm,
            },
            user: String::new(),
            pass: String::new(),
            code: String::new(),
            agree: false,
            error: None,
            session: None,
            tokens: None,
            linking: None,
            device_id: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
    pub(crate) fn busy(&self) -> bool {
        matches!(self.step, Step::Busy(_) | Step::Waiting(_))
    }
}

impl App {
    // ---------------- Dialogs: sign-in, enrolment, removal ----------------

    pub(crate) fn open_flow(&mut self, kind: FlowKind) {
        if let Some(f) = &self.flow {
            f.cancel.store(true, Ordering::Relaxed);
        }
        self.flow_gen += 1;
        let mut f = Flow::new(kind, self.flow_gen);
        match kind {
            FlowKind::Relogin(id) => f.user = self.account(id).map(|a| a.field("account_name").to_string()).unwrap_or_default(),
            FlowKind::Revoke(id) => f.code = self.account(id).map(|a| a.field("revocation_code").to_string()).unwrap_or_default(),
            FlowKind::Settings => {
                self.settings = SettingsForm::default();
            }
            _ => {}
        }
        self.flow = Some(f);
    }

    pub(crate) fn close_flow(&mut self) {
        if let Some(f) = self.flow.take() {
            f.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Runs work off-thread and delivers the result back into the current dialog.
    pub(crate) fn spawn(&self, ctx: &egui::Context, job: impl FnOnce() -> FlowEv + Send + 'static) {
        let Some(f) = &self.flow else { return };
        let (tx, ctx, generation) = (self.tx.clone(), ctx.clone(), f.generation);
        std::thread::spawn(move || {
            let ev = job();
            let _ = tx.send(Msg::Flow(generation, ev));
            ctx.request_repaint();
        });
    }

    fn set_step(&mut self, step: Step) {
        if let Some(f) = &mut self.flow {
            f.step = step;
        }
    }

    fn flow_error(&mut self, step: Step, e: impl Into<String>) {
        if let Some(f) = &mut self.flow {
            f.step = step;
            f.error = Some(e.into());
        }
    }

    pub(crate) fn start_login(&mut self, ctx: &egui::Context) {
        let Some(f) = &mut self.flow else { return };
        f.error = None;
        f.step = Step::Busy(t("Contacting Steam…", "Связываемся со Steam…").into());
        let (user, pass) = (f.user.trim().to_string(), f.pass.clone());
        self.spawn(ctx, move || FlowEv::Begun(crate::api::begin_login(&user, &pass)));
    }

    fn start_polling(&mut self, ctx: &egui::Context) {
        let Some(f) = &self.flow else { return };
        let Some(s) = f.session.clone() else { return };
        let cancel = f.cancel.clone();
        self.spawn(ctx, move || {
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return FlowEv::LoggedIn(Err(ApiError { eresult: 0, message: t("Cancelled", "Отменено").into() }));
                }
                match crate::api::poll(&s) {
                    Ok(Some(t)) => return FlowEv::LoggedIn(Ok(t)),
                    Ok(None) => {}
                    Err(e) => return FlowEv::LoggedIn(Err(e)),
                }
                if Instant::now() > deadline {
                    return FlowEv::LoggedIn(Err(ApiError {
                        eresult: 0,
                        message: t("Timed out — try again", "Время ожидания истекло — попробуйте ещё раз").into(),
                    }));
                }
                std::thread::sleep(Duration::from_secs_f32(s.interval.clamp(1.0, 10.0)));
            }
        });
    }

    pub(crate) fn submit_code(&mut self, ctx: &egui::Context, code: String, kind: Guard) {
        let Some(s) = self.flow.as_ref().and_then(|f| f.session.clone()) else { return };
        if let Some(f) = &mut self.flow {
            f.error = None;
        }
        self.set_step(Step::Busy(t("Checking the code…", "Проверяем код…").into()));
        self.spawn(ctx, move || FlowEv::CodeSent(crate::api::submit_code(&s, &code, kind)));
    }

    pub(crate) fn start_finalize(&mut self, ctx: &egui::Context) {
        let Some(f) = &mut self.flow else { return };
        let (Some(tokens), Some((steamid, secret, ..))) = (f.tokens.clone(), f.linking.clone()) else { return };
        let activation = f.code.trim().to_string();
        f.error = None;
        f.step = Step::Busy(t("Activating the authenticator…", "Активируем аутентификатор…").into());
        let start = self.steam_time();
        self.spawn(ctx, move || {
            let mut time = start;
            for _ in 0..30 {
                let code = crate::steam::generate_code(&secret, time).unwrap_or_default();
                match crate::api::finalize(&tokens.access_token, steamid, &code, time, &activation) {
                    Ok(None) => return FlowEv::Finalized(Ok(())),
                    Ok(Some(server)) => time = if server > time { server } else { time + 30 },
                    Err(e) => return FlowEv::Finalized(Err(e)),
                }
            }
            FlowEv::Finalized(Err(ApiError {
                eresult: 0,
                message: t("Steam never accepted the codes — try again", "Steam так и не принял коды — попробуйте ещё раз").into(),
            }))
        });
    }

    pub(crate) fn start_revoke(&mut self, ctx: &egui::Context, steamid: u64) {
        let Some(acc) = self.account(steamid) else { return };
        let mut auth = Self::auth_for(acc);
        let Some(f) = &mut self.flow else { return };
        let code = f.code.trim().to_string();
        f.error = None;
        f.step = Step::Busy(t("Removing the authenticator…", "Отвязываем аутентификатор…").into());
        self.spawn(ctx, move || {
            let r = (|| {
                auth.ensure_fresh()?;
                match crate::api::remove_authenticator(&auth.access_token, &code) {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(left)) => {
                        Err(tf!("Wrong revocation code. Attempts left: {left}", "Неверный код восстановления. Осталось попыток: {left}"))
                    }
                    Err(e) => Err(e.human()),
                }
            })();
            FlowEv::Removed(r)
        });
    }

    pub(crate) fn on_flow(&mut self, ctx: &egui::Context, generation: u64, ev: FlowEv) {
        let Some(f) = &self.flow else { return };
        if f.generation != generation {
            return; // the dialog is already closed, or was reopened
        }
        let kind = f.kind;
        match ev {
            FlowEv::Begun(Err(e)) => self.flow_error(Step::Credentials, e.human()),
            FlowEv::Begun(Ok(session)) => {
                // Which confirmation methods Steam is offering.
                let has = |g: Guard| session.guards.iter().any(|(k, _)| *k == g);
                let (email, device, dev_conf, email_conf) =
                    (has(Guard::EmailCode), has(Guard::DeviceCode), has(Guard::DeviceConfirmation), has(Guard::EmailConfirmation));
                let hint = session.guards.iter().find(|(_, h)| !h.is_empty()).map(|(_, h)| h.clone()).unwrap_or_default();
                if let FlowKind::Relogin(id) = kind {
                    // With a known shared_secret the authenticator code can go straight out.
                    if device && let Some(secret) = self.account(id).map(|a| a.field("shared_secret").to_string()).filter(|s| !s.is_empty())
                    {
                        let code = crate::steam::generate_code(&secret, self.steam_time()).unwrap_or_default();
                        if let Some(f) = &mut self.flow {
                            f.session = Some(session);
                        }
                        self.submit_code(ctx, code, Guard::DeviceCode);
                        return;
                    }
                }
                if let Some(f) = &mut self.flow {
                    f.session = Some(session);
                }
                if email {
                    self.set_step(Step::EmailCode(hint));
                } else if device {
                    self.set_step(Step::EmailCode(String::new())); // the code comes from the Steam Guard app
                } else if dev_conf || email_conf {
                    let msg = if dev_conf {
                        t("Confirm the sign-in in the Steam mobile app", "Подтвердите вход в мобильном приложении Steam").to_string()
                    } else {
                        tf!(
                            "Confirm the sign-in with the link from the email{}",
                            "Подтвердите вход по ссылке из письма{}",
                            if hint.is_empty() { String::new() } else { format!(" ({hint})") }
                        )
                    };
                    self.set_step(Step::Waiting(msg));
                    self.start_polling(ctx);
                } else {
                    // No second factor at all, so go straight to polling.
                    self.set_step(Step::Waiting(t("Finishing the sign-in…", "Завершаем вход…").into()));
                    self.start_polling(ctx);
                }
            }
            FlowEv::CodeSent(Err(e)) => {
                let hint = self
                    .flow
                    .as_ref()
                    .and_then(|f| f.session.as_ref())
                    .and_then(|s| s.guards.iter().find(|(g, h)| *g == Guard::EmailCode && !h.is_empty()).map(|(_, h)| h.clone()))
                    .unwrap_or_default();
                self.flow_error(Step::EmailCode(hint), e.human());
            }
            FlowEv::CodeSent(Ok(())) => {
                self.set_step(Step::Waiting(t("Finishing the sign-in…", "Завершаем вход…").into()));
                self.start_polling(ctx);
            }
            FlowEv::LoggedIn(Err(e)) => {
                if e.message == t("Cancelled", "Отменено") {
                    return;
                }
                self.flow_error(Step::Credentials, e.human());
            }
            FlowEv::LoggedIn(Ok(tokens)) => match kind {
                FlowKind::Relogin(id) => {
                    if tokens.refresh_token.is_empty() {
                        self.flow_error(
                            Step::Credentials,
                            t("Steam returned no token — try again", "Steam не вернул токен — попробуйте ещё раз"),
                        );
                        return;
                    }
                    if let Some(mut acc) = self.account(id).cloned() {
                        acc.set_tokens(&tokens.access_token, Some(&tokens.refresh_token));
                        acc.raw["Session"]["SessionID"] = Value::String(random_session_id());
                        self.save_acc(acc);
                    }
                    self.set_step(Step::Done(t("Done. The session was refreshed.", "Готово! Сессия обновлена.").into()));
                    self.confs.remove(&id);
                    self.load_confs(ctx, id);
                }
                FlowKind::Link => {
                    // Signed in; now create the authenticator.
                    if let Some(f) = &mut self.flow {
                        f.tokens = Some(tokens.clone());
                        f.device_id = crate::api::new_device_id();
                        f.step = Step::Busy(t("Creating the authenticator…", "Создаём аутентификатор…").into());
                    }
                    let device = self.flow.as_ref().map(|f| f.device_id.clone()).unwrap_or_default();
                    let token = tokens.access_token.clone();
                    // The steamid comes from the auth session.
                    let steamid = self.flow.as_ref().and_then(|f| f.session.as_ref()).map(|s| s.steamid).unwrap_or(0);
                    self.spawn(ctx, move || FlowEv::Added(crate::api::add_authenticator(&token, steamid, &device)));
                }
                _ => {}
            },
            FlowEv::Added(Err(e)) => {
                let msg = if e.eresult == 2 || e.message.to_lowercase().contains("phone") {
                    t(
                        "This account has no phone number. Add one at store.steampowered.com (Security) and try again.",
                        "К аккаунту не привязан телефон. Добавьте номер на store.steampowered.com (Безопасность) и повторите.",
                    )
                    .to_string()
                } else {
                    e.human()
                };
                self.flow_error(Step::Credentials, msg);
            }
            FlowEv::Added(Ok(auth)) => {
                let steamid = self.flow.as_ref().and_then(|f| f.session.as_ref()).map(|s| s.steamid).unwrap_or(0);
                let tokens = self.flow.as_ref().and_then(|f| f.tokens.clone());
                let device = self.flow.as_ref().map(|f| f.device_id.clone()).unwrap_or_default();
                // Save the maFile immediately. If activation fails afterwards, the
                // revocation code would otherwise be lost with it.
                let raw = build_mafile(&auth, steamid, &device, tokens.as_ref());
                if let Some(store) = &mut self.store {
                    match store.add_raw(raw, steamid) {
                        Ok(_) => {}
                        Err(e) => {
                            self.flow_error(Step::Credentials, tf!("Could not save the maFile: {e}", "Не удалось сохранить maFile: {e}"));
                            return;
                        }
                    }
                }
                self.reload_accounts();
                if let Some(f) = &mut self.flow {
                    f.linking = Some((
                        steamid,
                        auth.shared_secret.clone(),
                        auth.revocation_code.clone(),
                        auth.confirm_type,
                        auth.phone_hint.clone(),
                    ));
                    f.code.clear();
                    f.step = Step::Activate;
                }
            }
            FlowEv::Finalized(Err(e)) => self.flow_error(Step::Activate, e.human()),
            FlowEv::Finalized(Ok(())) => {
                let steamid = self.flow.as_ref().and_then(|f| f.linking.as_ref()).map(|(id, ..)| *id).unwrap_or(0);
                if let Some(mut acc) = self.account(steamid).cloned() {
                    acc.raw["fully_enrolled"] = Value::Bool(true);
                    self.save_acc(acc);
                }
                self.set_step(Step::Done(
                    t("Authenticator added. Save the revocation code.", "Аутентификатор привязан! Сохраните код восстановления.").into(),
                ));
                self.selected = None;
                self.select(ctx, steamid);
            }
            FlowEv::Removed(Err(e)) => self.flow_error(Step::Confirm, e),
            FlowEv::Removed(Ok(())) => {
                let steamid = if let FlowKind::Revoke(id) = kind { id } else { 0 };
                if let Some(acc) = self.account(steamid).cloned()
                    && let Some(store) = &mut self.store
                {
                    let _ = store.archive(&acc);
                }
                self.toast(t("Authenticator removed", "Аутентификатор отвязан"), false);
                self.close_flow();
                self.selected = None;
                self.reload_accounts();
            }
        }
    }
}

/// Builds the maFile JSON in the original SDA's format.
fn build_mafile(a: &NewAuthenticator, steamid: u64, device_id: &str, tokens: Option<&Tokens>) -> Value {
    let (access, refresh) = tokens.map(|t| (t.access_token.as_str(), t.refresh_token.as_str())).unwrap_or(("", ""));
    json!({
        "shared_secret": a.shared_secret,
        "serial_number": a.serial_number.to_string(),
        "revocation_code": a.revocation_code,
        "uri": a.uri,
        "server_time": a.server_time,
        "account_name": a.account_name,
        "token_gid": a.token_gid,
        "identity_secret": a.identity_secret,
        "secret_1": a.secret_1,
        "status": a.status,
        "device_id": device_id,
        "fully_enrolled": false,
        "Session": {
            "SteamID": steamid,
            "AccessToken": access,
            "RefreshToken": refresh,
            "SessionID": random_session_id(),
        }
    })
}
