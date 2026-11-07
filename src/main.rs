#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod config;
mod flow;
mod i18n;
mod icon;
mod modals;
mod proto;
mod screens;
mod steam;
mod storage;
mod theme;
mod tray;
mod widgets;

use config::{load_config, mafiles_dir, save_mafiles_dir, startup_lang};
use eframe::egui::{self, Align, Color32, Layout, RichText};
use flow::{Flow, FlowEv};
use i18n::t;
use modals::SettingsForm;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use steam::{Auth, Confirmation, NewTokens};
use storage::{Account, MetaMap, Store};
use theme::*;
use tray::{Tray, TrayAccount};

// ---------------- State ----------------

enum Msg {
    TimeOffset(Result<i64, String>),
    Confs { steamid: u64, result: Result<Vec<Confirmation>, String>, tokens: NewTokens },
    Acted { steamid: u64, ids: Vec<String>, accept: bool, result: Result<(), String>, tokens: NewTokens },
    Flow(u64, FlowEv),
    Profile { steamid: u64, persona: String, avatar: String, image: Option<egui::ColorImage> },
}

#[derive(Default)]
struct ConfState {
    loaded: bool,
    loading: bool,
    list: Vec<Confirmation>,
    error: Option<String>,
    acting: HashSet<String>,
}

#[derive(PartialEq)]
enum Screen {
    Unlock,
    Main,
    Fatal(String),
}

struct Toast {
    text: String,
    error: bool,
    born: f64,
}

struct App {
    store: Option<Store>,
    screen: Screen,
    accounts: Vec<Account>,
    meta: MetaMap,
    meta_dirty: Option<f64>,
    selected: Option<u64>,
    search: String,
    passkey_input: String,
    unlock_error: Option<String>,
    time_offset: i64,
    syncing: bool,
    confs: HashMap<u64, ConfState>,
    show_revocation: bool,
    editing_alias: bool,
    dragging: Option<u64>,
    enrich: bool,
    avatars: HashMap<u64, egui::TextureHandle>,
    enriched: HashSet<u64>,
    blur: bool,
    blur_applied: Option<bool>,
    tray: Option<Tray>,
    /// The window handle. The tray hides and restores the window through Win32 directly.
    hwnd: isize,
    minimize_to_tray: bool,
    /// Fingerprint of what the tray already has, so the menu is not resent every frame.
    tray_sig: u64,
    tray_offset: Option<i64>,
    tray_locked: Option<bool>,
    flow: Option<Flow>,
    flow_gen: u64,
    settings: SettingsForm,
    toasts: Vec<Toast>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    styled: bool,
    copied_at: f64,
}

fn random_session_id() -> String {
    let mut b = [0u8; 12];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl App {
    fn new() -> Self {
        let (tx, rx) = channel();
        let cfg = load_config();
        i18n::set(startup_lang(&cfg));
        let mut app = App {
            store: None,
            screen: Screen::Main,
            accounts: Vec::new(),
            meta: MetaMap::new(),
            meta_dirty: None,
            selected: None,
            search: String::new(),
            passkey_input: String::new(),
            unlock_error: None,
            time_offset: 0,
            syncing: false,
            confs: HashMap::new(),
            show_revocation: false,
            editing_alias: false,
            dragging: None,
            enrich: cfg.enrich,
            avatars: HashMap::new(),
            enriched: HashSet::new(),
            blur: cfg.blur,
            blur_applied: None,
            tray: Tray::spawn(),
            hwnd: 0,
            minimize_to_tray: cfg.minimize_to_tray,
            tray_sig: 0,
            tray_offset: None,
            tray_locked: None,
            flow: None,
            flow_gen: 0,
            settings: SettingsForm::default(),
            toasts: Vec::new(),
            tx,
            rx,
            styled: false,
            copied_at: -10.0,
        };
        match Store::open(mafiles_dir()) {
            Ok(store) => {
                app.screen = if store.needs_passkey() { Screen::Unlock } else { Screen::Main };
                app.store = Some(store);
                if app.screen == Screen::Main {
                    app.reload_accounts();
                }
            }
            Err(e) => app.screen = Screen::Fatal(e),
        }
        app
    }

    fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { text: text.into(), error, born: f64::NAN });
    }

    fn alias(&self, steamid: u64) -> &str {
        self.meta.get(&steamid).map(|m| m.alias.trim()).unwrap_or("")
    }

    /// The profile persona name, if the lookup setting is on and it has been fetched.
    fn persona(&self, steamid: u64) -> &str {
        if !self.enrich {
            return "";
        }
        self.meta.get(&steamid).map(|m| m.persona.trim()).unwrap_or("")
    }

    /// Display name, in order of preference: alias, then persona name, then login.
    fn display_name(&self, acc: &Account) -> String {
        let a = self.alias(acc.steamid);
        if !a.is_empty() {
            return a.to_string();
        }
        let p = self.persona(acc.steamid);
        if !p.is_empty() { p.to_string() } else { acc.name().to_string() }
    }

    fn sort_accounts(&mut self) {
        let mut accs = std::mem::take(&mut self.accounts);
        // Manually ordered accounts first, everything else by name.
        accs.sort_by_cached_key(|a| {
            let order = self.meta.get(&a.steamid).map(|m| m.order).unwrap_or(0);
            let key = if order == 0 { i32::MAX } else { order };
            (key, self.display_name(a).to_lowercase())
        });
        self.accounts = accs;
    }

    /// Applies a new order, numbering the given steamids 1..=n and saving it.
    fn apply_order(&mut self, ordered: &[u64]) {
        for (i, id) in ordered.iter().enumerate() {
            self.meta.entry(*id).or_default().order = i as i32 + 1;
        }
        self.meta_dirty = Some(0.0); // save on the next frame
        self.sort_accounts();
    }

    fn reload_accounts(&mut self) {
        let Some(store) = &self.store else { return };
        let (accs, errs) = store.load_all();
        self.meta = store.load_meta();
        self.accounts = accs;
        self.sort_accounts();
        if !self.selected.is_some_and(|s| self.accounts.iter().any(|a| a.steamid == s)) {
            self.selected = self.accounts.first().map(|a| a.steamid);
        }
        for e in errs {
            self.toast(e, true);
        }
    }

    fn current(&self) -> Option<&Account> {
        let id = self.selected?;
        self.accounts.iter().find(|a| a.steamid == id)
    }

    fn account(&self, steamid: u64) -> Option<&Account> {
        self.accounts.iter().find(|a| a.steamid == steamid)
    }

    fn select(&mut self, ctx: &egui::Context, steamid: u64) {
        if self.selected == Some(steamid) {
            return;
        }
        self.flush_meta();
        self.selected = Some(steamid);
        self.show_revocation = false;
        self.editing_alias = false;
        if !self.confs.get(&steamid).is_some_and(|s| s.loaded) {
            self.load_confs(ctx, steamid);
        }
    }

    fn flush_meta(&mut self) {
        if self.meta_dirty.take().is_none() {
            return;
        }
        if let Some(store) = &self.store
            && let Err(e) = store.save_meta(&self.meta)
        {
            self.toast(tf!("Could not save notes: {e}", "Не удалось сохранить заметки: {e}"), true);
        }
    }

    fn sync_time(&mut self, ctx: &egui::Context) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::TimeOffset(steam::query_time_offset()));
            ctx.request_repaint();
        });
    }

    /// Fetches persona name and avatar for every account, in the background.
    fn enrich_all(&mut self, ctx: &egui::Context) {
        if !self.enrich {
            return;
        }
        let ids: Vec<u64> = self.accounts.iter().map(|a| a.steamid).collect();
        for id in ids {
            if self.enriched.insert(id) {
                self.enrich_one(ctx, id);
            }
        }
    }

    fn enrich_one(&self, ctx: &egui::Context, steamid: u64) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let (persona, avatar) = steam::fetch_profile(steamid).unwrap_or_default();
            let image = (!avatar.is_empty())
                .then(|| steam::fetch_bytes(&avatar).ok())
                .flatten()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|img| {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)
                });
            let _ = tx.send(Msg::Profile { steamid, persona, avatar, image });
            ctx.request_repaint();
        });
    }

    fn auth_for(acc: &Account) -> Auth {
        let session_id = acc.session("SessionID");
        Auth {
            steamid: acc.steamid,
            device_id: acc.field("device_id").to_string(),
            identity_secret: acc.field("identity_secret").to_string(),
            access_token: acc.session("AccessToken").to_string(),
            refresh_token: acc.session("RefreshToken").to_string(),
            session_id: if session_id.is_empty() { random_session_id() } else { session_id.to_string() },
        }
    }

    fn load_confs(&mut self, ctx: &egui::Context, steamid: u64) {
        let Some(acc) = self.account(steamid) else { return };
        let mut auth = Self::auth_for(acc);
        let state = self.confs.entry(steamid).or_default();
        if state.loading {
            return;
        }
        state.loading = true;
        state.error = None;
        let (tx, ctx, offset) = (self.tx.clone(), ctx.clone(), self.time_offset);
        std::thread::spawn(move || {
            let (result, tokens) = steam::fetch_confirmations(&mut auth, offset);
            let _ = tx.send(Msg::Confs { steamid, result, tokens });
            ctx.request_repaint();
        });
    }

    fn act(&mut self, ctx: &egui::Context, steamid: u64, items: Vec<(String, String)>, accept: bool) {
        let Some(acc) = self.account(steamid) else { return };
        let mut auth = Self::auth_for(acc);
        let state = self.confs.entry(steamid).or_default();
        let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
        state.acting.extend(ids.iter().cloned());
        let (tx, ctx, offset) = (self.tx.clone(), ctx.clone(), self.time_offset);
        std::thread::spawn(move || {
            let (result, tokens) = steam::respond(&mut auth, offset, &items, accept);
            let _ = tx.send(Msg::Acted { steamid, ids, accept, result, tokens });
            ctx.request_repaint();
        });
    }

    fn save_acc(&mut self, acc: Account) {
        if let Some(store) = &mut self.store
            && let Err(e) = store.save_account(&acc)
        {
            self.toast(tf!("Could not save the maFile: {e}", "Не удалось сохранить maFile: {e}"), true);
        }
        if let Some(a) = self.accounts.iter_mut().find(|a| a.steamid == acc.steamid) {
            *a = acc;
        }
    }

    fn store_tokens(&mut self, steamid: u64, tokens: NewTokens) {
        let Some((access, refresh)) = tokens else { return };
        let Some(mut acc) = self.account(steamid).cloned() else { return };
        acc.set_tokens(&access, refresh.as_deref());
        self.save_acc(acc);
    }

    fn steam_time(&self) -> i64 {
        steam::unix_now() + self.time_offset
    }

    /// Hands the tray the current accounts, time offset and lock state. Called every
    /// frame but only sends what changed, since the tray rebuilds its menu on receipt.
    fn sync_tray(&mut self) {
        let Some(tray) = &self.tray else { return };

        if self.tray_offset != Some(self.time_offset) {
            self.tray_offset = Some(self.time_offset);
            tray.set_time_offset(self.time_offset);
        }
        let locked = self.screen == Screen::Unlock;
        if self.tray_locked != Some(locked) {
            self.tray_locked = Some(locked);
            tray.set_locked(locked);
        }

        // Secrets never change, so only ids and labels go into the fingerprint.
        let items: Vec<(u64, String)> = self.accounts.iter().map(|a| (a.steamid, self.display_name(a))).collect();
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            items.hash(&mut h);
            h.finish()
        };
        if sig == self.tray_sig {
            return;
        }
        self.tray_sig = sig;
        let accounts = items
            .into_iter()
            .filter_map(|(steamid, label)| {
                let acc = self.accounts.iter().find(|a| a.steamid == steamid)?;
                Some(TrayAccount { steamid, label, secret: acc.field("shared_secret").to_string() })
            })
            .collect();
        tray.set_accounts(accounts);
    }

    fn import(&mut self, ctx: &egui::Context) {
        let Some(files) = rfd::FileDialog::new()
            .set_title(t("Choose a maFile", "Выберите maFile"))
            .add_filter("Steam Guard (maFile)", &["maFile", "mafile", "json"])
            .pick_files()
        else {
            return;
        };
        let Some(store) = &mut self.store else { return };
        let mut ok = Vec::new();
        let mut errs = Vec::new();
        for f in files {
            match store.import(&f) {
                Ok(a) => ok.push(a.steamid),
                Err(e) => errs.push(e),
            }
        }
        self.reload_accounts();
        if let Some(id) = ok.last() {
            self.toast(tf!("Accounts imported: {}", "Импортировано аккаунтов: {}", ok.len()), false);
            self.selected = None;
            self.select(ctx, *id);
        }
        for e in errs {
            self.toast(e, true);
        }
    }

    /// The maFiles directory of the open vault.
    fn current_dir(&self) -> PathBuf {
        self.store.as_ref().map(|s| s.dir.clone()).unwrap_or_else(mafiles_dir)
    }

    /// Picks a different maFiles directory and reopens the vault from it.
    fn choose_dir(&mut self, ctx: &egui::Context) {
        let Some(dir) =
            rfd::FileDialog::new().set_title(t("Folder for maFiles", "Папка для maFiles")).set_directory(self.current_dir()).pick_folder()
        else {
            return;
        };
        self.switch_dir(ctx, dir);
    }

    fn switch_dir(&mut self, ctx: &egui::Context, dir: PathBuf) {
        if dir == self.current_dir() {
            return;
        }
        self.flush_meta();
        match Store::open(dir.clone()) {
            Ok(store) => {
                if let Err(e) = save_mafiles_dir(&dir) {
                    self.toast(
                        tf!("Folder opened, but the path could not be remembered: {e}", "Папка открыта, но не удалось запомнить путь: {e}"),
                        true,
                    );
                }
                let needs = store.needs_passkey();
                self.store = Some(store);
                self.accounts.clear();
                self.confs.clear();
                self.selected = None;
                self.search.clear();
                self.close_flow();
                if needs {
                    self.screen = Screen::Unlock;
                } else {
                    self.screen = Screen::Main;
                    self.reload_accounts();
                    if let Some(id) = self.selected {
                        self.load_confs(ctx, id);
                    }
                }
                self.toast(tf!("maFiles folder: {}", "Папка maFiles: {}", dir.display()), false);
            }
            Err(e) => self.toast(tf!("Could not open the folder: {e}", "Не удалось открыть папку: {e}"), true),
        }
    }

    fn poll(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::TimeOffset(r) => {
                    self.syncing = false;
                    match r {
                        Ok(off) => self.time_offset = off,
                        Err(e) => self.toast(e, true),
                    }
                }
                Msg::Confs { steamid, result, tokens } => {
                    self.store_tokens(steamid, tokens);
                    let st = self.confs.entry(steamid).or_default();
                    st.loading = false;
                    st.loaded = true;
                    match result {
                        Ok(list) => {
                            st.list = list;
                            st.error = None;
                        }
                        Err(e) => st.error = Some(e),
                    }
                }
                Msg::Acted { steamid, ids, accept, result, tokens } => {
                    self.store_tokens(steamid, tokens);
                    let st = self.confs.entry(steamid).or_default();
                    for id in &ids {
                        st.acting.remove(id);
                    }
                    match result {
                        Ok(()) => {
                            st.list.retain(|c| !ids.contains(&c.id));
                            let what = if accept { t("Accepted", "Принято") } else { t("Rejected", "Отклонено") };
                            self.toast(format!("{what}: {}", ids.len()), false);
                        }
                        Err(e) => self.toast(e, true),
                    }
                }
                Msg::Flow(generation, ev) => {
                    // Background steps need a ctx of their own, outside the frame.
                    let ctx = CTX.with(|c| c.borrow().clone());
                    if let Some(ctx) = ctx {
                        self.on_flow(&ctx, generation, ev);
                    }
                }
                Msg::Profile { steamid, persona, avatar, image } => {
                    if let Some(img) = image
                        && let Some(ctx) = CTX.with(|c| c.borrow().clone())
                    {
                        let tex = ctx.load_texture(format!("av{steamid}"), img, egui::TextureOptions::LINEAR);
                        self.avatars.insert(steamid, tex);
                    }
                    // Cache persona and avatar into the metadata when they change.
                    let m = self.meta.entry(steamid).or_default();
                    if (!persona.is_empty() && m.persona != persona) || (!avatar.is_empty() && m.avatar != avatar) {
                        if !persona.is_empty() {
                            m.persona = persona;
                        }
                        if !avatar.is_empty() {
                            m.avatar = avatar;
                        }
                        self.meta_dirty = Some(0.0);
                        self.sort_accounts();
                    }
                }
            }
        }
    }
}

thread_local! {
    static CTX: std::cell::RefCell<Option<egui::Context>> = const { std::cell::RefCell::new(None) };
}

// ---------------- Screens ----------------

/// The tray needs the window handle: it shows and hides the window itself, without
/// waiting for a frame.
#[cfg(windows)]
fn window_hwnd(frame: &eframe::Frame) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(w)) => w.hwnd.get(),
        _ => 0,
    }
}

#[cfg(not(windows))]
fn window_hwnd(_: &eframe::Frame) -> isize {
    0
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        if !self.styled {
            apply_theme(&ctx);
            self.styled = true;
            self.sync_time(&ctx);
            if let Some(id) = self.selected {
                self.load_confs(&ctx, id);
            }
        }
        if self.hwnd == 0 {
            self.hwnd = window_hwnd(frame);
            if let Some(tray) = &self.tray {
                tray.set_window(self.hwnd);
            }
        }
        self.poll();
        self.enrich_all(&ctx);
        self.sync_tray();
        ctx.request_repaint_after(Duration::from_millis(250));

        // Notes are saved a beat after typing stops, not on every keystroke.
        if let Some(t) = self.meta_dirty {
            if ctx.input(|i| i.time) - t > 0.8 {
                self.flush_meta();
            } else {
                ctx.request_repaint_after(Duration::from_millis(400));
            }
        }

        // Apply or drop the acrylic backdrop whenever the setting changes.
        if self.blur_applied != Some(self.blur) {
            let _ = if self.blur {
                window_vibrancy::apply_acrylic(&*frame, Some((18, 20, 24, 170)))
            } else {
                window_vibrancy::clear_acrylic(&*frame)
            };
            self.blur_applied = Some(self.blur);
        }
        // The background is filled with square corners on purpose: DWM draws the rounded
        // corners and the hairline border itself, so top and bottom match.
        let win = ui.max_rect();
        let bg = if self.blur { Color32::from_rgba_unmultiplied(13, 14, 17, 140) } else { BG };
        ui.painter().rect_filled(win, 0.0, bg);
        let inner = win.shrink(1.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
            ui.set_min_size(inner.size());
            ui.set_clip_rect(inner);
            self.title_bar(ui);
            match &self.screen {
                Screen::Fatal(e) => {
                    let e = e.clone();
                    ui.vertical_centered(|ui| {
                        ui.add_space(120.0);
                        ui.label(
                            RichText::new(t("Could not open the maFiles folder", "Не удалось открыть папку maFiles")).size(20.0).color(RED),
                        );
                        ui.label(RichText::new(e).color(MUTED));
                    });
                }
                Screen::Unlock => self.unlock_screen(ui),
                Screen::Main => {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        self.sidebar(ui);
                        self.content(ui);
                    });
                }
            }
        });
        self.flow_modal(&ctx);
        self.draw_toasts(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_meta();
        // Remove the icon explicitly, or it lingers in the tray until hovered.
        if let Some(tray) = self.tray.take() {
            tray.shutdown();
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Steam Desktop Authenticator")
            .with_inner_size([940.0, 640.0])
            .with_min_inner_size([780.0, 500.0])
            .with_decorations(false)
            .with_transparent(true)
            // The .exe icon comes from a resource (build.rs); this one is for the
            // taskbar and Alt+Tab.
            .with_icon(egui::IconData { rgba: icon::shield_rgba(64), width: 64, height: 64 }),
        ..Default::default()
    };
    eframe::run_native("Steam Desktop Authenticator", options, Box::new(|_| Ok(Box::new(App::new()))))
}
