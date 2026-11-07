use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }
}

static ACTIVE: AtomicU8 = AtomicU8::new(Lang::En as u8);

pub fn set(lang: Lang) {
    ACTIVE.store(lang as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if ACTIVE.load(Ordering::Relaxed) == Lang::Ru as u8 { Lang::Ru } else { Lang::En }
}

/// Picks one of two literals for the active language.
pub fn t(en: &'static str, ru: &'static str) -> &'static str {
    match get() {
        Lang::En => en,
        Lang::Ru => ru,
    }
}

/// `t` for messages that interpolate values. Both arms are real format strings,
/// so inline captures such as `{e}` work the same in either language.
#[macro_export]
macro_rules! tf {
    ($en:literal, $ru:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::get() {
            $crate::i18n::Lang::En => format!($en $(, $arg)*),
            $crate::i18n::Lang::Ru => format!($ru $(, $arg)*),
        }
    };
}

#[cfg(windows)]
pub fn system_default() -> Lang {
    // The low 10 bits of a LANGID are the primary language; 0x19 is Russian.
    let langid = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    if langid & 0x3ff == 0x19 { Lang::Ru } else { Lang::En }
}

#[cfg(not(windows))]
pub fn system_default() -> Lang {
    Lang::En
}
