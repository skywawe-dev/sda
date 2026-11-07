//! The system tray icon.
//!
//! This runs on its own thread with its own Win32 message pump, because on Windows
//! `tray-icon` requires a message loop on whichever thread created the icon.
//! `GetMessageW` blocks until something arrives; the main thread wakes it with
//! `PostThreadMessageW` after pushing a command into the channel.
//!
//! Everything the tray does happens on that thread: showing the window, generating
//! the code, writing to the clipboard. That is deliberate. A window minimised to the
//! tray is hidden with `ShowWindow(SW_HIDE)`, so it receives no `WM_PAINT` and
//! `eframe::App::ui()` stops being called — there is no main-thread frame to defer to.

/// An account as the tray knows it: what to show, and what to derive the code from.
#[derive(Clone)]
pub struct TrayAccount {
    pub steamid: u64,
    pub label: String,
    pub secret: String,
}

#[cfg(windows)]
mod imp {
    use super::TrayAccount;
    use crate::i18n::t;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, IsIconic, MSG, PostMessageW, PostThreadMessageW, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow,
        ShowWindow, TranslateMessage, WM_CLOSE, WM_USER,
    };

    /// Our own wake-up message, meaning "there is a command in the channel".
    const WM_WAKE: u32 = WM_USER + 0x41;

    const ID_SHOW: &str = "show";
    const ID_QUIT: &str = "quit";

    enum Cmd {
        Accounts(Vec<TrayAccount>),
        TimeOffset(i64),
        Window(isize),
        Locked(bool),
        Shutdown,
    }

    pub struct Tray {
        tx: Sender<Cmd>,
        thread_id: u32,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl Tray {
        /// Creates the tray icon. `None` if the tray thread could not register it.
        pub fn spawn() -> Option<Tray> {
            let (tx, rx) = channel();
            let (ready_tx, ready_rx) = channel();
            let join = std::thread::Builder::new().name("tray".into()).spawn(move || run(rx, ready_tx)).ok()?;
            // Wait for the thread to report its id, which also means the icon exists.
            match ready_rx.recv() {
                Ok(thread_id) => Some(Tray { tx, thread_id, join: Some(join) }),
                Err(_) => None,
            }
        }

        fn send(&self, cmd: Cmd) {
            if self.tx.send(cmd).is_ok() {
                unsafe { PostThreadMessageW(self.thread_id, WM_WAKE, 0, 0) };
            }
        }

        pub fn set_accounts(&self, accounts: Vec<TrayAccount>) {
            self.send(Cmd::Accounts(accounts));
        }

        pub fn set_time_offset(&self, offset: i64) {
            self.send(Cmd::TimeOffset(offset));
        }

        pub fn set_window(&self, hwnd: isize) {
            self.send(Cmd::Window(hwnd));
        }

        /// While the vault is locked the tray has no secrets, and the menu says so.
        pub fn set_locked(&self, locked: bool) {
            self.send(Cmd::Locked(locked));
        }

        /// Removes the icon before the process exits; otherwise it lingers in the tray
        /// until something makes the shell notice, such as hovering over it.
        pub fn shutdown(mut self) {
            self.send(Cmd::Shutdown);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    struct State {
        icon: TrayIcon,
        accounts: Vec<TrayAccount>,
        offset: i64,
        hwnd: isize,
        locked: bool,
    }

    fn run(rx: Receiver<Cmd>, ready: Sender<u32>) {
        let icon = TrayIconBuilder::new()
            .with_tooltip("Steam Desktop Authenticator")
            .with_icon(shield_icon())
            // The menu is rebuilt on every click, otherwise its codes would be stale.
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(false)
            .build();
        let Ok(icon) = icon else { return };
        if ready.send(unsafe { GetCurrentThreadId() }).is_err() {
            return;
        }
        let mut st = State { icon, accounts: Vec::new(), offset: 0, hwnd: 0, locked: false };
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if r <= 0 {
                break; // 0 is WM_QUIT, -1 is an error
            }
            if msg.message == WM_WAKE {
                let mut stop = false;
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        Cmd::Accounts(v) => st.accounts = v,
                        Cmd::TimeOffset(o) => st.offset = o,
                        Cmd::Window(h) => st.hwnd = h,
                        Cmd::Locked(l) => st.locked = l,
                        Cmd::Shutdown => stop = true,
                    }
                }
                if stop {
                    break;
                }
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // Tray and menu events are pushed into their channels from inside the window
            // procedure, i.e. during DispatchMessageW, so drain them right afterwards.
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                st.on_tray(ev);
            }
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                st.on_menu(ev);
            }
        }
        drop(st.icon);
    }

    impl State {
        fn on_tray(&mut self, ev: TrayIconEvent) {
            match ev {
                // React on button release; the press arrives as its own event.
                TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } => match button {
                    MouseButton::Left => restore_window(self.hwnd),
                    MouseButton::Right => self.popup(),
                    MouseButton::Middle => {}
                },
                TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => restore_window(self.hwnd),
                _ => {}
            }
        }

        fn on_menu(&mut self, ev: MenuEvent) {
            let id = ev.id.0.as_str();
            if id == ID_SHOW {
                restore_window(self.hwnd);
            } else if id == ID_QUIT {
                // WM_CLOSE reaches winit even for a hidden window, so eframe shuts down cleanly.
                unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
            } else if let Some(id) = id.strip_prefix("acc:") {
                let Ok(steamid) = id.parse::<u64>() else { return };
                let Some(acc) = self.accounts.iter().find(|a| a.steamid == steamid) else { return };
                let Some(code) = crate::steam::generate_code(&acc.secret, self.now()) else { return };
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(code);
                }
            }
        }

        fn now(&self) -> i64 {
            crate::steam::unix_now() + self.offset
        }

        /// Rebuilds the menu with current codes and pops it up next to the icon.
        fn popup(&mut self) {
            let menu = Menu::new();
            let show = MenuItem::with_id(ID_SHOW, t("Show window", "Показать окно"), true, None);
            let quit = MenuItem::with_id(ID_QUIT, t("Quit", "Выход"), true, None);
            let sep1 = PredefinedMenuItem::separator();
            let sep2 = PredefinedMenuItem::separator();

            let now = self.now();
            let accounts: Vec<MenuItem> = if self.locked {
                let text = t("Vault is locked — open the window", "Хранилище зашифровано — откройте окно");
                vec![MenuItem::with_id("locked", text, false, None)]
            } else if self.accounts.is_empty() {
                vec![MenuItem::with_id("empty", t("No accounts", "Нет аккаунтов"), false, None)]
            } else {
                self.accounts
                    .iter()
                    .map(|a| {
                        let code = crate::steam::generate_code(&a.secret, now).unwrap_or_else(|| "?????".into());
                        // A \t in a Win32 menu item right-aligns everything after it.
                        let text = format!("{}\t{}", mnemonics_off(&a.label), code);
                        MenuItem::with_id(format!("acc:{}", a.steamid), text, true, None)
                    })
                    .collect()
            };

            let mut items: Vec<&dyn tray_icon::menu::IsMenuItem> = vec![&show, &sep1];
            items.extend(accounts.iter().map(|i| i as &dyn tray_icon::menu::IsMenuItem));
            items.push(&sep2);
            items.push(&quit);
            if menu.append_items(&items).is_err() {
                return;
            }
            self.icon.set_menu(Some(Box::new(menu)));
            self.icon.show_menu();
        }
    }

    /// `&` marks an accelerator in menu text, and Steam logins are allowed to contain one.
    fn mnemonics_off(s: &str) -> String {
        s.replace('&', "&&")
    }

    /// Hides the window. `SW_HIDE` rather than minimising, so it leaves the taskbar too.
    pub fn hide_window(hwnd: isize) {
        unsafe { ShowWindow(hwnd as HWND, SW_HIDE) };
    }

    pub fn restore_window(hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        let hwnd = hwnd as HWND;
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
    }

    fn shield_icon() -> Icon {
        const N: u32 = 32;
        Icon::from_rgba(crate::icon::shield_rgba(N), N, N).expect("a 32x32 shield is valid RGBA")
    }
}

#[cfg(not(windows))]
mod imp {
    use super::TrayAccount;

    /// Stub for non-Windows builds: the app runs, there is simply no tray.
    pub struct Tray;

    impl Tray {
        pub fn spawn() -> Option<Tray> {
            None
        }
        pub fn set_accounts(&self, _: Vec<TrayAccount>) {}
        pub fn set_time_offset(&self, _: i64) {}
        pub fn set_window(&self, _: isize) {}
        pub fn set_locked(&self, _: bool) {}
        pub fn shutdown(self) {}
    }

    pub fn hide_window(_: isize) {}
    pub fn restore_window(_: isize) {}
}

pub use imp::{Tray, hide_window};
