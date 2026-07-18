//! Assemble the stylesheet: the live theme tokens (current/colors.css) followed
//! by our bundled obsidian-glass rules, loaded as ONE provider so the
//! @define-color names resolve for our rules. If current/colors.css is missing
//! we prepend the obsidian defaults so the panel is always styled.

use gtk4::gdk::Display;
use gtk4::CssProvider;
use std::path::PathBuf;

const STYLE: &str = include_str!("../../../config/tezca-settings/style.css");

const FALLBACK_TOKENS: &str = "\
@define-color tz_base        #0B0E0F;
@define-color tz_surface     #14191B;
@define-color tz_text        #E8EAED;
@define-color tz_subtext     #C3C8CC;
@define-color tz_muted       #8B9398;
@define-color tz_faint       #5A6166;
@define-color tz_accent      #3FB8AF;
@define-color tz_accent_dim  #2A8C86;
@define-color tz_on_accent   #0B0E0F;
@define-color tz_gold        #C9A24B;
@define-color tz_urgent      #E06C75;
@define-color tz_on_urgent   #0B0E0F;
";

pub fn install() {
    let Some(display) = Display::default() else { return };
    let tokens = read_tokens().unwrap_or_else(|| FALLBACK_TOKENS.to_string());
    let provider = CssProvider::new();
    provider.load_from_data(&format!("{tokens}\n{STYLE}"));
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn read_tokens() -> Option<String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    std::fs::read_to_string(base.join("tezca/current/colors.css")).ok()
}
