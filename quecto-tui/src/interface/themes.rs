//! Named theme system — semantic color definitions for the TUI.
//!
//! Each theme defines a set of color functions that components use for styling.
//! Themes are statically defined (no runtime file loading in Phase 7).

/// A named theme with semantic color functions.
#[derive(Debug, Clone)]
pub struct NamedTheme {
    pub name: &'static str,
    /// Foreground colors (ANSI 256-color codes).
    pub accent: u8,
    pub muted: u8,
    pub dim: u8,
    pub success: u8,
    pub error: u8,
    pub warning: u8,
    pub info: u8,
    pub border: u8,
    pub tool: u8,
    pub heading: u8,
    pub code: u8,
    pub link: u8,
    pub quote: u8,
}

impl NamedTheme {
    pub fn fg(&self, color: u8, text: &str) -> String {
        format!("\x1b[38;5;{}m{}\x1b[0m", color, text)
    }

    pub fn accent(&self, text: &str) -> String {
        self.fg(self.accent, text)
    }

    pub fn muted(&self, text: &str) -> String {
        self.fg(self.muted, text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.fg(self.dim, text)
    }

    pub fn success(&self, text: &str) -> String {
        self.fg(self.success, text)
    }

    pub fn error(&self, text: &str) -> String {
        self.fg(self.error, text)
    }

    pub fn warning(&self, text: &str) -> String {
        self.fg(self.warning, text)
    }

    pub fn tool(&self, text: &str) -> String {
        self.fg(self.tool, text)
    }

    pub fn heading(&self, text: &str) -> String {
        self.fg(self.heading, text)
    }

    pub fn code(&self, text: &str) -> String {
        self.fg(self.code, text)
    }

    pub fn link(&self, text: &str) -> String {
        self.fg(self.link, text)
    }
}

/// Dark theme (default) — matches Quecto TUI's dark mode.
pub const DARK: NamedTheme = NamedTheme {
    name: "dark",
    accent: 6,   // cyan
    muted: 245,  // gray
    dim: 240,    // dark gray
    success: 2,  // green
    error: 1,    // red
    warning: 3,  // yellow
    info: 6,     // cyan
    border: 245, // gray
    tool: 4,     // blue
    heading: 6,  // cyan
    code: 6,     // cyan
    link: 4,     // blue
    quote: 245,  // gray
};

/// Light theme — brighter colors for light terminal backgrounds.
pub const LIGHT: NamedTheme = NamedTheme {
    name: "light",
    accent: 25,   // dark blue
    muted: 242,   // medium gray
    dim: 248,     // light gray
    success: 28,  // dark green
    error: 124,   // dark red
    warning: 130, // dark yellow/orange
    info: 25,     // dark blue
    border: 242,  // medium gray
    tool: 25,     // dark blue
    heading: 25,  // dark blue
    code: 90,     // purple
    link: 25,     // dark blue
    quote: 242,   // medium gray
};

/// All available themes.
pub const ALL_THEMES: &[&NamedTheme] = &[&DARK, &LIGHT];

/// Get a theme by name.
pub fn get_theme(name: &str) -> &'static NamedTheme {
    ALL_THEMES.iter().find(|t| t.name == name).unwrap_or(&&DARK)
}

/// List available theme names.
pub fn theme_names() -> Vec<&'static str> {
    ALL_THEMES.iter().map(|t| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_exists() {
        let t = get_theme("dark");
        assert_eq!(t.name, "dark");
    }

    #[test]
    fn light_theme_exists() {
        let t = get_theme("light");
        assert_eq!(t.name, "light");
    }

    #[test]
    fn unknown_theme_defaults_to_dark() {
        let t = get_theme("nonexistent");
        assert_eq!(t.name, "dark");
    }

    #[test]
    fn accent_applies_color() {
        let t = &DARK;
        let s = t.accent("hello");
        assert!(s.contains("\x1b[38;5;6m")); // cyan
        assert!(s.contains("hello"));
    }

    #[test]
    fn error_applies_red() {
        let t = &DARK;
        let s = t.error("fail");
        assert!(s.contains("\x1b[38;5;1m")); // red
    }

    #[test]
    fn theme_names_list() {
        let names = theme_names();
        assert!(names.contains(&"dark"));
        assert!(names.contains(&"light"));
    }

    #[test]
    fn light_theme_has_different_colors() {
        assert_ne!(DARK.accent, LIGHT.accent);
    }

    #[test]
    fn all_color_methods_produce_ansi() {
        let t = &DARK;
        type ColorMethod = (fn(&NamedTheme, &str) -> String, u8);
        let methods: Vec<ColorMethod> = vec![
            (NamedTheme::accent, t.accent),
            (NamedTheme::muted, t.muted),
            (NamedTheme::dim, t.dim),
            (NamedTheme::success, t.success),
            (NamedTheme::error, t.error),
            (NamedTheme::warning, t.warning),
            (NamedTheme::tool, t.tool),
            (NamedTheme::heading, t.heading),
            (NamedTheme::code, t.code),
            (NamedTheme::link, t.link),
        ];
        for (method, expected_code) in methods {
            let s = method(t, "x");
            assert!(
                s.contains(&format!("\x1b[38;5;{}m", expected_code)),
                "missing ANSI for color {}",
                expected_code
            );
            assert!(s.ends_with("\x1b[0m"), "missing reset");
        }
    }

    #[test]
    fn fg_with_arbitrary_color() {
        let t = &DARK;
        let s = t.fg(199, "pink");
        assert!(s.contains("\x1b[38;5;199m"));
        assert!(s.contains("pink"));
    }

    #[test]
    fn light_theme_all_methods() {
        let t = &LIGHT;
        assert!(t.accent("a").contains("a"));
        assert!(t.muted("m").contains("m"));
        assert!(t.dim("d").contains("d"));
        assert!(t.success("s").contains("s"));
        assert!(t.error("e").contains("e"));
        assert!(t.warning("w").contains("w"));
        assert!(t.tool("t").contains("t"));
        assert!(t.heading("h").contains("h"));
        assert!(t.code("c").contains("c"));
        assert!(t.link("l").contains("l"));
    }

    #[test]
    fn all_themes_have_unique_names() {
        let names = theme_names();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len());
    }
}
