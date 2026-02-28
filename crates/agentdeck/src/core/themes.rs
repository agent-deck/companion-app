//! Theme system with Ghostty theme support
//!
//! Parses Ghostty theme files and provides a registry of available themes.
//! Built-in "Dark" and "Light" themes are always available.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;
use wezterm_cell::color::RgbColor;
use wezterm_term::color::ColorPalette;

/// A terminal color theme
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub is_light: bool,
    pub palette: [(u8, u8, u8); 16],
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub cursor_color: (u8, u8, u8),
    pub cursor_text: (u8, u8, u8),
    pub selection_bg: (u8, u8, u8),
    pub selection_fg: (u8, u8, u8),
}

impl Theme {
    /// Convert this theme to a wezterm ColorPalette
    pub fn to_color_palette(&self) -> ColorPalette {
        let mut cp = ColorPalette::default();

        for (i, (r, g, b)) in self.palette.iter().enumerate() {
            cp.colors.0[i] = RgbColor::new_8bpc(*r, *g, *b).into();
        }

        cp.foreground = RgbColor::new_8bpc(self.foreground.0, self.foreground.1, self.foreground.2).into();
        cp.background = RgbColor::new_8bpc(self.background.0, self.background.1, self.background.2).into();
        cp.cursor_fg = RgbColor::new_8bpc(self.cursor_text.0, self.cursor_text.1, self.cursor_text.2).into();
        cp.cursor_bg = RgbColor::new_8bpc(self.cursor_color.0, self.cursor_color.1, self.cursor_color.2).into();
        cp.cursor_border = RgbColor::new_8bpc(self.cursor_color.0, self.cursor_color.1, self.cursor_color.2).into();
        cp.selection_fg = RgbColor::new_8bpc(self.selection_fg.0, self.selection_fg.1, self.selection_fg.2).into();
        cp.selection_bg = RgbColor::new_8bpc(self.selection_bg.0, self.selection_bg.1, self.selection_bg.2).into();

        cp
    }

    /// Get the COLORFGBG value for this theme
    pub fn colorfgbg(&self) -> String {
        if self.is_light {
            "0;15".to_string()
        } else {
            "15;0".to_string()
        }
    }

    /// Get background as egui Color32
    pub fn background_color32(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.background.0, self.background.1, self.background.2)
    }

    /// Get foreground as egui Color32
    pub fn foreground_color32(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.foreground.0, self.foreground.1, self.foreground.2)
    }

    /// Get selection background as egui Color32
    pub fn selection_bg_color32(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.selection_bg.0, self.selection_bg.1, self.selection_bg.2)
    }

    /// Get cursor color as egui Color32
    pub fn cursor_color32(&self) -> egui::Color32 {
        let (r, g, b) = self.cursor_color;
        egui::Color32::from_rgba_unmultiplied(r, g, b, 220)
    }
}

/// Parse a hex color string like "#ff00aa" into (r, g, b)
fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Compute whether a color is "light" based on perceived luminance
fn is_light_color(r: u8, g: u8, b: u8) -> bool {
    let luminance = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
    luminance > 128.0
}

/// Parse a Ghostty theme file into a Theme
pub fn parse_ghostty_theme(name: &str, content: &str) -> Option<Theme> {
    let mut palette = [(0u8, 0u8, 0u8); 16];
    let mut background = None;
    let mut foreground = None;
    let mut cursor_color = None;
    let mut cursor_text = None;
    let mut selection_bg = None;
    let mut selection_fg = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();

        match key {
            "palette" => {
                // Format: "N=#rrggbb"
                let mut pparts = value.splitn(2, '=');
                let idx_str = pparts.next()?.trim();
                let color_str = pparts.next()?.trim();
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx < 16 {
                        if let Some(color) = parse_hex_color(color_str) {
                            palette[idx] = color;
                        }
                    }
                }
            }
            "background" => background = parse_hex_color(value),
            "foreground" => foreground = parse_hex_color(value),
            "cursor-color" => cursor_color = parse_hex_color(value),
            "cursor-text" => cursor_text = parse_hex_color(value),
            "selection-background" => selection_bg = parse_hex_color(value),
            "selection-foreground" => selection_fg = parse_hex_color(value),
            _ => {}
        }
    }

    let bg = background?;
    let fg = foreground?;

    Some(Theme {
        name: name.to_string(),
        is_light: is_light_color(bg.0, bg.1, bg.2),
        palette,
        background: bg,
        foreground: fg,
        cursor_color: cursor_color.unwrap_or(fg),
        cursor_text: cursor_text.unwrap_or(bg),
        selection_bg: selection_bg.unwrap_or((70, 130, 180)),
        selection_fg: selection_fg.unwrap_or(fg),
    })
}

/// Load Ghostty themes from the application bundle
pub fn load_ghostty_themes() -> Vec<Theme> {
    let themes_dir = Path::new("/Applications/Ghostty.app/Contents/Resources/ghostty/themes");
    if !themes_dir.exists() {
        return Vec::new();
    }

    let mut themes = Vec::new();
    if let Ok(entries) = std::fs::read_dir(themes_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(theme) = parse_ghostty_theme(&name, &content) {
                        themes.push(theme);
                    }
                }
            }
        }
    }

    themes
}

/// Built-in "Dark" theme (wezterm default palette with our dark background)
fn built_in_dark() -> Theme {
    // Use wezterm's default ANSI colors
    let default_palette = ColorPalette::default();
    let mut palette = [(0u8, 0u8, 0u8); 16];
    for i in 0..16 {
        let srgba = default_palette.resolve_fg(wezterm_cell::color::ColorAttribute::PaletteIndex(i as u8));
        palette[i] = (
            (srgba.0 * 255.0) as u8,
            (srgba.1 * 255.0) as u8,
            (srgba.2 * 255.0) as u8,
        );
    }

    Theme {
        name: "Dark".to_string(),
        is_light: false,
        palette,
        background: (30, 30, 30),
        foreground: (220, 220, 220),
        cursor_color: (200, 200, 200),
        cursor_text: (30, 30, 30),
        selection_bg: (70, 130, 180),
        selection_fg: (255, 255, 255),
    }
}

/// Built-in "Light" theme (OneHalfLight colors)
fn built_in_light() -> Theme {
    Theme {
        name: "Light".to_string(),
        is_light: true,
        palette: [
            (0x38, 0x3a, 0x42), // 0  Black
            (0xe4, 0x56, 0x49), // 1  Red
            (0x50, 0xa1, 0x4f), // 2  Green
            (0xc1, 0x84, 0x01), // 3  Yellow
            (0x01, 0x84, 0xbc), // 4  Blue
            (0xa6, 0x26, 0xa4), // 5  Magenta
            (0x09, 0x97, 0xb3), // 6  Cyan
            (0xfa, 0xfa, 0xfa), // 7  White
            (0x4f, 0x52, 0x5e), // 8  Bright Black
            (0xe0, 0x6c, 0x75), // 9  Bright Red
            (0x98, 0xc3, 0x79), // 10 Bright Green
            (0xe5, 0xc0, 0x7b), // 11 Bright Yellow
            (0x61, 0xaf, 0xef), // 12 Bright Blue
            (0xc6, 0x78, 0xdd), // 13 Bright Magenta
            (0x56, 0xb6, 0xc2), // 14 Bright Cyan
            (0xff, 0xff, 0xff), // 15 Bright White
        ],
        background: (250, 250, 250),
        foreground: (56, 58, 66),
        cursor_color: (191, 206, 255),
        cursor_text: (56, 58, 66),
        selection_bg: (191, 206, 255),
        selection_fg: (56, 58, 66),
    }
}

/// Get the path to ~/.claude.json using platform-native home directory
fn claude_json_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

/// Read Claude Code's theme preference from ~/.claude.json.
/// Returns true if the theme starts with "light", false otherwise.
pub fn read_claude_theme_is_light() -> bool {
    let path = match claude_json_path() {
        Some(p) => p,
        None => return false,
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    match parsed.get("theme").and_then(|v| v.as_str()) {
        Some(theme) => theme.starts_with("light"),
        None => false,
    }
}

/// Get the modification time of ~/.claude.json (for change detection)
pub fn claude_json_mtime() -> Option<SystemTime> {
    let path = claude_json_path()?;
    std::fs::metadata(&path).ok()?.modified().ok()
}

/// Registry of all available themes
pub struct ThemeRegistry {
    themes: Vec<Theme>,
    by_name: HashMap<String, usize>,
}

impl ThemeRegistry {
    /// Create a new registry, loading built-in and Ghostty themes
    pub fn new() -> Self {
        let mut themes = Vec::new();

        // Built-in themes first
        themes.push(built_in_dark());
        themes.push(built_in_light());

        // Load Ghostty themes
        let ghostty = load_ghostty_themes();
        for theme in ghostty {
            // Skip if name conflicts with built-in
            if theme.name == "Dark" || theme.name == "Light" {
                continue;
            }
            themes.push(theme);
        }

        // Sort: built-ins first (Dark, Light), then alphabetically
        themes[2..].sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let by_name: HashMap<String, usize> = themes
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i))
            .collect();

        Self { themes, by_name }
    }

    /// Find a theme by name
    pub fn find(&self, name: &str) -> Option<&Theme> {
        self.by_name.get(name).map(|&i| &self.themes[i])
    }

    /// Get all themes
    pub fn all(&self) -> &[Theme] {
        &self.themes
    }

    /// Map old ColorScheme value to theme name
    pub fn theme_name_from_color_scheme(scheme: &str) -> &str {
        match scheme {
            "Dark" => "Dark",
            "Light" => "Light",
            _ => "Dark",
        }
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("#00ff00"), Some((0, 255, 0)));
        assert_eq!(parse_hex_color("0000ff"), Some((0, 0, 255)));
        assert_eq!(parse_hex_color("#fafafa"), Some((250, 250, 250)));
        assert_eq!(parse_hex_color("bad"), None);
    }

    #[test]
    fn test_is_light_color() {
        assert!(is_light_color(250, 250, 250)); // Light bg
        assert!(is_light_color(255, 255, 255)); // White
        assert!(!is_light_color(30, 30, 30)); // Dark bg
        assert!(!is_light_color(0, 0, 0)); // Black
    }

    #[test]
    fn test_parse_ghostty_theme() {
        let content = r#"
palette = 0=#21222c
palette = 1=#ff5555
palette = 2=#50fa7b
palette = 3=#f1fa8c
palette = 4=#bd93f9
palette = 5=#ff79c6
palette = 6=#8be9fd
palette = 7=#f8f8f2
palette = 8=#6272a4
palette = 9=#ff6e6e
palette = 10=#69ff94
palette = 11=#ffffa5
palette = 12=#d6acff
palette = 13=#ff92df
palette = 14=#a4ffff
palette = 15=#ffffff
background = #282a36
foreground = #f8f8f2
cursor-color = #f8f8f2
cursor-text = #282a36
selection-background = #44475a
selection-foreground = #ffffff
"#;
        let theme = parse_ghostty_theme("Dracula", content).unwrap();
        assert_eq!(theme.name, "Dracula");
        assert!(!theme.is_light);
        assert_eq!(theme.background, (0x28, 0x2a, 0x36));
        assert_eq!(theme.foreground, (0xf8, 0xf8, 0xf2));
        assert_eq!(theme.palette[1], (0xff, 0x55, 0x55));
    }

    #[test]
    fn test_built_in_themes() {
        let dark = built_in_dark();
        assert_eq!(dark.name, "Dark");
        assert!(!dark.is_light);

        let light = built_in_light();
        assert_eq!(light.name, "Light");
        assert!(light.is_light);
    }

    #[test]
    fn test_theme_registry() {
        let registry = ThemeRegistry::new();
        assert!(registry.find("Dark").is_some());
        assert!(registry.find("Light").is_some());
        // Built-ins are always present
        assert!(registry.all().len() >= 2);
    }

    #[test]
    fn test_theme_to_color_palette() {
        let theme = built_in_dark();
        let palette = theme.to_color_palette();
        // Just verify it doesn't panic and returns something
        let _ = palette.resolve_fg(wezterm_cell::color::ColorAttribute::Default);
    }

    #[test]
    fn test_colorfgbg() {
        let dark = built_in_dark();
        assert_eq!(dark.colorfgbg(), "15;0");

        let light = built_in_light();
        assert_eq!(light.colorfgbg(), "0;15");
    }

    #[test]
    fn test_backward_compat_mapping() {
        assert_eq!(ThemeRegistry::theme_name_from_color_scheme("Dark"), "Dark");
        assert_eq!(ThemeRegistry::theme_name_from_color_scheme("Light"), "Light");
        assert_eq!(ThemeRegistry::theme_name_from_color_scheme("Unknown"), "Dark");
    }
}
