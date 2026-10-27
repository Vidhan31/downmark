//! Background discovery and classification of system fonts.

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::App;

pub(crate) const DEFAULT_FONT_LABEL: &str = "Default";

/// Registry that searches and classifies system fonts asynchronously.
pub(crate) struct FontRegistry;

static SYSTEM_FONTS: RwLock<Vec<String>> = RwLock::new(Vec::new());
static MONO_FONTS: RwLock<Vec<String>> = RwLock::new(Vec::new());
static REGISTRY_INITIALIZED: AtomicBool = AtomicBool::new(false);
static FONTS_LOADED: AtomicBool = AtomicBool::new(false);

impl FontRegistry {
    /// Kicks off background asynchronous scanning of all system fonts on startup.
    pub(crate) fn init(cx: &mut App) {
        if REGISTRY_INITIALIZED.swap(true, Ordering::SeqCst) {
            return;
        }

        let text_system = std::sync::Arc::clone(cx.text_system());
        let executor = cx.background_executor().clone();

        executor
            .spawn(async move {
                let mut raw_names = text_system.all_font_names();
                raw_names.sort_unstable();
                raw_names.dedup();

                let mut clean_fonts = Vec::with_capacity(raw_names.len() + 1);
                clean_fonts.push(DEFAULT_FONT_LABEL.to_string());

                let mut clean_mono = Vec::new();
                clean_mono.push(DEFAULT_FONT_LABEL.to_string());

                for name in raw_names {
                    if name.starts_with('.') || name.trim().is_empty() {
                        continue;
                    }
                    if is_monospace_font_name(&name) {
                        clean_mono.push(name.clone());
                    }
                    clean_fonts.push(name);
                }

                if let Ok(mut lock) = SYSTEM_FONTS.write() {
                    *lock = clean_fonts;
                }
                if let Ok(mut lock) = MONO_FONTS.write() {
                    *lock = clean_mono;
                }

                FONTS_LOADED.store(true, Ordering::SeqCst);
            })
            .detach();
    }

    /// Returns whether the asynchronous font scanning has completed.
    pub(crate) fn is_loaded() -> bool {
        FONTS_LOADED.load(Ordering::Relaxed)
    }

    /// Returns all known system fonts with "Default" at the beginning.
    pub(crate) fn all_fonts() -> Vec<String> {
        if let Ok(lock) = SYSTEM_FONTS.read()
            && !lock.is_empty()
        {
            return lock.clone();
        }
        vec![
            DEFAULT_FONT_LABEL.to_string(),
            "Cantarell".to_string(),
            "DejaVu Sans".to_string(),
            "Inter".to_string(),
            "Liberation Sans".to_string(),
            "Noto Sans".to_string(),
            "Roboto".to_string(),
            "Ubuntu".to_string(),
        ]
    }

    /// Returns detected monospace fonts prioritized, followed by the rest of the system fonts.
    pub(crate) fn code_fonts() -> Vec<String> {
        let mono_list = if let Ok(lock) = MONO_FONTS.read() {
            lock.clone()
        } else {
            Vec::new()
        };

        if !mono_list.is_empty() {
            let all = Self::all_fonts();
            let mut result = mono_list;
            let seen: std::collections::HashSet<_> = result.iter().cloned().collect();
            for font in all {
                if !seen.contains(&font) {
                    result.push(font);
                }
            }
            return result;
        }

        vec![
            DEFAULT_FONT_LABEL.to_string(),
            "Cascadia Mono".to_string(),
            "Consolas".to_string(),
            "Courier New".to_string(),
            "DejaVu Sans Mono".to_string(),
            "Fira Code".to_string(),
            "JetBrains Mono".to_string(),
            "Liberation Mono".to_string(),
            "Menlo".to_string(),
            "Monaco".to_string(),
            "Noto Sans Mono".to_string(),
            "Source Code Pro".to_string(),
            "Ubuntu Mono".to_string(),
        ]
    }
}

/// Helper identifying common monospace font naming patterns across Linux, macOS, and Windows.
pub(crate) fn is_monospace_font_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("mono")
        || lower.contains("code")
        || lower.contains("courier")
        || lower.contains("consolas")
        || lower.contains("menlo")
        || lower.contains("cascadia")
        || lower.contains("fixed")
        || lower.contains("terminal")
        || lower.contains("typewriter")
        || lower.contains("inconsolata")
        || lower.contains("hack")
        || lower.contains("iosevka")
        || lower.contains("terminus")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monospace_font_detection() {
        assert!(is_monospace_font_name("JetBrains Mono"));
        assert!(is_monospace_font_name("Fira Code"));
        assert!(is_monospace_font_name("DejaVu Sans Mono"));
        assert!(is_monospace_font_name("Courier New"));
        assert!(is_monospace_font_name("Consolas"));
        assert!(is_monospace_font_name("Menlo"));
        assert!(is_monospace_font_name("Cascadia Code"));
        assert!(is_monospace_font_name("Source Code Pro"));
        assert!(is_monospace_font_name("Ubuntu Mono"));

        assert!(!is_monospace_font_name("Inter"));
        assert!(!is_monospace_font_name("Roboto"));
        assert!(!is_monospace_font_name("Cantarell"));
        assert!(!is_monospace_font_name("Helvetica"));
        assert!(!is_monospace_font_name("Georgia"));
    }

    #[test]
    fn test_default_fallbacks() {
        let all = FontRegistry::all_fonts();
        assert_ne!(all, Vec::<String>::new());
        assert_eq!(all[0], DEFAULT_FONT_LABEL);

        let code = FontRegistry::code_fonts();
        assert_ne!(code, Vec::<String>::new());
        assert_eq!(code[0], DEFAULT_FONT_LABEL);
    }
}
