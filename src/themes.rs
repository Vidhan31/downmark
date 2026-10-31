//! Themes bundled with Downmark and the application-wide preferences global.

use std::sync::OnceLock;

use gpui_kit::{
    App, Hsla, Rgba, SharedString, Window,
    base::TypographyTokens,
    component::{ActiveTheme as _, Theme, ThemeMode, ThemeRegistry},
    px,
};

use crate::{
    diagram::DiagramTheme,
    settings::{Appearance, Settings},
};

static DEFAULT_MONO_FONT: OnceLock<SharedString> = OnceLock::new();

/// Theme sets shipped in the binary, from the GPUI Kit theme collection.
const BUNDLED_THEMES: &[&str] = &[
    include_str!("../themes/ayu.json"),
    include_str!("../themes/catppuccin.json"),
    include_str!("../themes/everforest.json"),
    include_str!("../themes/flexoki.json"),
    include_str!("../themes/gruvbox.json"),
    include_str!("../themes/macos-classic.json"),
    include_str!("../themes/solarized.json"),
    include_str!("../themes/tokyonight.json"),
];

pub(crate) use crate::settings::AppSettings;

pub(crate) fn init(settings: Settings, cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for theme_set in BUNDLED_THEMES {
        if let Err(err) = registry.load_themes_from_str(theme_set) {
            #[expect(
                clippy::print_stderr,
                reason = "early startup logging for bundled theme loading"
            )]
            {
                eprintln!("downmark: could not load a bundled theme: {err}");
            }
        }
    }
    DEFAULT_MONO_FONT.get_or_init(|| Theme::global(cx).mono_font_family.clone());
    cx.set_global(AppSettings(settings));
    apply(None, cx);
}

/// Apply the preferred themes, appearance and interface size.
///
/// `window` supplies the system appearance when the preference is to follow
/// it; without one the application-wide appearance is used.
pub(crate) fn apply(window: Option<&Window>, cx: &mut App) {
    let settings = AppSettings::get(cx).clone();
    let registry = ThemeRegistry::global(cx);
    let pick = |name: &Option<String>, mode: ThemeMode| {
        name.as_deref()
            .and_then(|name| registry.themes().get(name))
            .filter(|theme| theme.mode == mode)
            .cloned()
            .or_else(|| registry.default_themes().get(&mode).cloned())
            .or_else(|| {
                registry
                    .themes()
                    .values()
                    .find(|theme| theme.mode == mode)
                    .cloned()
            })
            .unwrap_or_else(|| std::rc::Rc::new(gpui_kit::component::ThemeConfig::default()))
    };
    let light = pick(&settings.light_theme, ThemeMode::Light);
    let dark = pick(&settings.dark_theme, ThemeMode::Dark);

    let mode = match settings.appearance {
        Appearance::Light => ThemeMode::Light,
        Appearance::Dark => ThemeMode::Dark,
        Appearance::System => window
            .map_or_else(|| cx.window_appearance(), Window::appearance)
            .into(),
    };

    Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
    Theme::change(mode, None, cx);
    let default_mono = DEFAULT_MONO_FONT
        .get()
        .cloned()
        .unwrap_or_else(|| TypographyTokens::default().mono);
    Theme::update(cx, |theme| {
        theme.font_size = px(settings.font_size);
        if let Some(ref family) = settings.font_family {
            theme.font_family = family.as_str().into();
        } else {
            theme.font_family = ".SystemUIFont".into();
        }
        if let Some(ref mono) = settings.mono_font_family {
            theme.mono_font_family = mono.as_str().into();
        } else {
            theme.mono_font_family = default_mono;
        }
    });
}

/// Names of the registered themes for `mode`, sorted for display.
pub(crate) fn theme_names(mode: ThemeMode, cx: &App) -> Vec<SharedString> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|theme| theme.mode == mode)
        .map(|theme| theme.name.clone())
        .collect()
}

/// Remember `name` as the theme for its mode and switch to that mode.
pub(crate) fn select_theme(name: &SharedString, cx: &mut App) {
    let Some(mode) = ThemeRegistry::global(cx)
        .themes()
        .get(name)
        .map(|theme| theme.mode)
    else {
        return;
    };
    // Following the system stays in effect when the theme suits the current
    // appearance; otherwise switching to the theme means switching modes.
    let follows_system =
        AppSettings::get(cx).appearance == Appearance::System && Theme::global(cx).mode == mode;
    AppSettings::update(cx, |settings| {
        if mode.is_dark() {
            settings.dark_theme = Some(name.to_string());
        } else {
            settings.light_theme = Some(name.to_string());
        }
        if !follows_system {
            settings.appearance = if mode.is_dark() {
                Appearance::Dark
            } else {
                Appearance::Light
            };
        }
    });
}

/// `color` as `#rrggbb`, composited over `background` when translucent.
fn hex_over(color: Hsla, background: Hsla) -> String {
    let (fg, bg) = (Rgba::from(color), Rgba::from(background));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "Float multiplied by 255. and clamped to [0, 1] safely fits in u8"
    )]
    let mix = |f: f32, b: f32| ((f * fg.a + b * (1. - fg.a)).clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        mix(fg.r, bg.r),
        mix(fg.g, bg.g),
        mix(fg.b, bg.b)
    )
}

/// The diagram colors that match the app's theme.
pub(crate) fn diagram_theme(cx: &App) -> DiagramTheme {
    let theme = cx.theme();
    let background = theme.background;
    let hex = |color: Hsla| hex_over(color, background);
    // Shades are tints of the text color over the background, so every
    // theme gets light fills and clear outlines, whatever its own
    // secondary colors are tuned for.
    let tint = |amount: f32| hex(theme.foreground.opacity(amount));
    DiagramTheme {
        dark: theme.is_dark(),
        canvas: hex(background),
        surface: tint(0.045),
        surface_alt: tint(0.09),
        text: hex(theme.foreground),
        subtle_text: hex(theme.muted_foreground),
        border: tint(0.35),
        line: tint(0.55),
        note_background: hex(theme.warning.opacity(0.16)),
        note_border: hex(theme.warning.opacity(0.7)),
        error: hex(theme.danger),
        warning: hex(theme.warning),
        success: hex(theme.success),
        // Distinct hues (themes' chart colors are often shades of one),
        // softened toward the background.
        series: [
            theme.blue,
            theme.green,
            theme.yellow,
            theme.red,
            theme.magenta,
            theme.cyan,
        ]
        .map(|color| hex(color.opacity(0.75)))
        .to_vec(),
        background: hex(background),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composites_translucent_colors() {
        let white: Hsla = gpui_kit::rgb(0x00ff_ffff).into();
        let black: Hsla = gpui_kit::rgb(0x0000_0000).into();
        assert_eq!(hex_over(black, white), "#000000");
        assert_eq!(hex_over(black.opacity(0.5), white), "#808080");
        assert_eq!(
            hex_over(gpui_kit::rgb(0x0009_69da).into(), black),
            "#0969da"
        );
    }
}
