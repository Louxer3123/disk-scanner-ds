//! The language of what the backend tells the user: error messages and the
//! Explorer menu entry. The UI sets it at start and when the user switches.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// The message in the current language.
pub fn tr(ru: &'static str, en: &'static str) -> &'static str {
    if english() { en } else { ru }
}

/// `ru` or `en`; anything else means Russian.
#[tauri::command]
pub fn set_language(lang: String) {
    ENGLISH.store(lang == "en", Ordering::Relaxed);
}
