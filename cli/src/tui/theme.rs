//! Theme system + terminal capability detection.
//!
//! Built-ins stay professional by default (`midnight`); the rest are one
//! `/theme` away. Custom themes live at `~/.config/local-ai/theme.json`.
//! `NO_COLOR` forces monochrome; `--ascii` (or no UTF-8 locale) degrades
//! the anime faces through [`crate::tui::anime::ascii_line`].

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// `ratatui::Color` as `"red"` / `"light-cyan"` / `"#ff00aa"` for theme.json.
mod color_string {
    use ratatui::style::Color;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(color: &Color, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::color_name(color))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Color, D::Error> {
        let name = String::deserialize(d)?;
        Ok(super::parse_color(&name))
    }
}

fn color_name(color: &Color) -> String {
    match color {
        Color::Black => "black",
        Color::Red => "red",
        Color::Green => "green",
        Color::Yellow => "yellow",
        Color::Blue => "blue",
        Color::Magenta => "magenta",
        Color::Cyan => "cyan",
        Color::Gray => "gray",
        Color::DarkGray => "dark-gray",
        Color::LightRed => "light-red",
        Color::LightGreen => "light-green",
        Color::LightYellow => "light-yellow",
        Color::LightBlue => "light-blue",
        Color::LightMagenta => "light-magenta",
        Color::LightCyan => "light-cyan",
        Color::White => "white",
        Color::Rgb(r, g, b) => return format!("#{:02x}{:02x}{:02x}", r, g, b),
        _ => "white",
    }
    .to_string()
}

fn parse_color(name: &str) -> Color {
    if let Some(hex) = name.strip_prefix('#') {
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return Color::Rgb(r, g, b);
            }
        }
    }
    match name.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark-gray" | "dark-grey" => Color::DarkGray,
        "light-red" => Color::LightRed,
        "light-green" => Color::LightGreen,
        "light-yellow" => Color::LightYellow,
        "light-blue" => Color::LightBlue,
        "light-magenta" => Color::LightMagenta,
        "light-cyan" => Color::LightCyan,
        "white" => Color::White,
        _ => Color::White,
    }
}

/// Full palette for one theme. Colors are ratatui colors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(with = "color_string")]
    pub header: Color,
    #[serde(with = "color_string")]
    pub accent: Color,
    #[serde(with = "color_string")]
    pub success: Color,
    #[serde(with = "color_string")]
    pub warning: Color,
    #[serde(with = "color_string")]
    pub error: Color,
    #[serde(with = "color_string")]
    pub muted: Color,
    #[serde(with = "color_string")]
    pub border: Color,
    #[serde(with = "color_string")]
    pub character: Color,
}

impl Default for Theme {
    fn default() -> Self {
        builtin("midnight")
    }
}

/// Built-in themes: midnight (default) · cyberpunk · anime · matrix ·
/// monochrome · minimal.
pub fn builtin(name: &str) -> Theme {
    match name.to_lowercase().as_str() {
        "cyberpunk" => Theme {
            name: "cyberpunk".into(),
            header: Color::Magenta,
            accent: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            muted: Color::DarkGray,
            border: Color::Magenta,
            character: Color::Cyan,
        },
        "anime" => Theme {
            name: "anime".into(),
            header: Color::LightMagenta,
            accent: Color::LightCyan,
            success: Color::LightGreen,
            warning: Color::LightYellow,
            error: Color::LightRed,
            muted: Color::Gray,
            border: Color::LightMagenta,
            character: Color::LightCyan,
        },
        "matrix" => Theme {
            name: "matrix".into(),
            header: Color::Green,
            accent: Color::LightGreen,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            muted: Color::DarkGray,
            border: Color::Green,
            character: Color::LightGreen,
        },
        "monochrome" => Theme {
            name: "monochrome".into(),
            header: Color::White,
            accent: Color::Gray,
            success: Color::White,
            warning: Color::Gray,
            error: Color::White,
            muted: Color::DarkGray,
            border: Color::Gray,
            character: Color::White,
        },
        "minimal" => Theme {
            name: "minimal".into(),
            header: Color::Gray,
            accent: Color::White,
            success: Color::Gray,
            warning: Color::Gray,
            error: Color::Gray,
            muted: Color::DarkGray,
            border: Color::DarkGray,
            character: Color::Gray,
        },
        _ => Theme {
            name: "midnight".into(),
            header: Color::Blue,
            accent: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            muted: Color::DarkGray,
            border: Color::Blue,
            character: Color::Cyan,
        },
    }
}

pub fn builtin_names() -> Vec<&'static str> {
    vec!["midnight", "cyberpunk", "anime", "matrix", "monochrome", "minimal"]
}

fn custom_theme_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|b| b.join("local-ai").join("theme.json"))
}

/// Load `~/.config/local-ai/theme.json` when present. Unknown fields are
/// ignored so partial overrides work.
pub fn load_custom() -> Option<Theme> {
    let path = custom_theme_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// What the terminal can do (checked once at startup).
#[derive(Debug, Clone)]
pub struct TermCaps {
    /// Colors allowed (`NO_COLOR` forces false).
    pub colors: bool,
    /// Unicode faces allowed (`--ascii` or non-UTF-8 locale forces false).
    pub unicode: bool,
    /// Animation frames allowed (`animation = off` forces false).
    pub animation: bool,
}

pub fn detect_caps(ascii: bool, no_animation: bool) -> TermCaps {
    let no_color = std::env::var("NO_COLOR").map(|v| !v.is_empty()).unwrap_or(false);
    let term = std::env::var("TERM").unwrap_or_default();
    let colors = !no_color && term != "dumb";
    let lang = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default()
        .to_uppercase();
    let unicode = !ascii && (lang.contains("UTF-8") || lang.contains("UTF8") || lang.is_empty());
    TermCaps { colors, unicode, animation: !no_animation }
}
