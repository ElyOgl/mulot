//! Color themes. Every widget pulls its colors from the active [`Theme`].

use clap::ValueEnum;
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ThemeName {
    Aurora,
    Nord,
    Gruvbox,
    Dracula,
    /// 16-color palette that works in every terminal.
    Ansi,
    Mono,
}

impl ThemeName {
    pub const ALL: [ThemeName; 6] = [
        ThemeName::Aurora,
        ThemeName::Nord,
        ThemeName::Gruvbox,
        ThemeName::Dracula,
        ThemeName::Ansi,
        ThemeName::Mono,
    ];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Picks a truecolor theme when the terminal advertises 24-bit color and
    /// falls back to the 16-color palette otherwise.
    pub fn detect() -> Self {
        let colorterm = std::env::var("COLORTERM")
            .unwrap_or_default()
            .to_lowercase();
        let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
        let term = std::env::var("TERM").unwrap_or_default();
        let truecolor = colorterm.contains("truecolor")
            || colorterm.contains("24bit")
            || matches!(
                program.as_str(),
                "iTerm.app" | "WezTerm" | "ghostty" | "vscode" | "WarpTerminal" | "Hyper" | "Tabby"
            )
            || term.contains("kitty")
            || term.contains("alacritty")
            || term.contains("ghostty");
        if truecolor {
            ThemeName::Aurora
        } else {
            ThemeName::Ansi
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub fg: Color,
    pub muted: Color,
    pub border: Color,
    pub title: Color,
    pub accent: Color,
    pub accent_fg: Color,
    pub cpu: Color,
    pub mem: Color,
    pub swap: Color,
    pub rx: Color,
    pub tx: Color,
    pub disk: Color,
    pub gpu: Color,
    pub ok: Color,
    pub warn: Color,
    pub crit: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub track: Color,
}

impl Theme {
    pub fn get(name: ThemeName) -> Theme {
        match name {
            ThemeName::Aurora => Theme {
                name: "aurora",
                fg: rgb(0xe6e9f5),
                muted: rgb(0x7a819c),
                border: rgb(0x3b4261),
                title: rgb(0xc0caf5),
                accent: rgb(0x7dcfff),
                accent_fg: rgb(0x16161e),
                cpu: rgb(0x7dcfff),
                mem: rgb(0xbb9af7),
                swap: rgb(0xff9e64),
                rx: rgb(0x9ece6a),
                tx: rgb(0xf7768e),
                disk: rgb(0xe0af68),
                gpu: rgb(0x2ac3de),
                ok: rgb(0x9ece6a),
                warn: rgb(0xe0af68),
                crit: rgb(0xf7768e),
                selection_bg: rgb(0x2e3c64),
                selection_fg: rgb(0xffffff),
                track: rgb(0x2a2f45),
            },
            ThemeName::Nord => Theme {
                name: "nord",
                fg: rgb(0xeceff4),
                muted: rgb(0x7b88a1),
                border: rgb(0x434c5e),
                title: rgb(0x88c0d0),
                accent: rgb(0x88c0d0),
                accent_fg: rgb(0x2e3440),
                cpu: rgb(0x88c0d0),
                mem: rgb(0xb48ead),
                swap: rgb(0xd08770),
                rx: rgb(0xa3be8c),
                tx: rgb(0xbf616a),
                disk: rgb(0xebcb8b),
                gpu: rgb(0x81a1c1),
                ok: rgb(0xa3be8c),
                warn: rgb(0xebcb8b),
                crit: rgb(0xbf616a),
                selection_bg: rgb(0x434c5e),
                selection_fg: rgb(0xeceff4),
                track: rgb(0x3b4252),
            },
            ThemeName::Gruvbox => Theme {
                name: "gruvbox",
                fg: rgb(0xebdbb2),
                muted: rgb(0x928374),
                border: rgb(0x504945),
                title: rgb(0xfabd2f),
                accent: rgb(0xfabd2f),
                accent_fg: rgb(0x282828),
                cpu: rgb(0x83a598),
                mem: rgb(0xd3869b),
                swap: rgb(0xfe8019),
                rx: rgb(0xb8bb26),
                tx: rgb(0xfb4934),
                disk: rgb(0xfabd2f),
                gpu: rgb(0x8ec07c),
                ok: rgb(0xb8bb26),
                warn: rgb(0xfabd2f),
                crit: rgb(0xfb4934),
                selection_bg: rgb(0x504945),
                selection_fg: rgb(0xfbf1c7),
                track: rgb(0x3c3836),
            },
            ThemeName::Dracula => Theme {
                name: "dracula",
                fg: rgb(0xf8f8f2),
                muted: rgb(0x6272a4),
                border: rgb(0x44475a),
                title: rgb(0xbd93f9),
                accent: rgb(0xff79c6),
                accent_fg: rgb(0x282a36),
                cpu: rgb(0x8be9fd),
                mem: rgb(0xbd93f9),
                swap: rgb(0xffb86c),
                rx: rgb(0x50fa7b),
                tx: rgb(0xff5555),
                disk: rgb(0xf1fa8c),
                gpu: rgb(0xff79c6),
                ok: rgb(0x50fa7b),
                warn: rgb(0xf1fa8c),
                crit: rgb(0xff5555),
                selection_bg: rgb(0x44475a),
                selection_fg: rgb(0xf8f8f2),
                track: rgb(0x343746),
            },
            ThemeName::Ansi => Theme {
                name: "ansi",
                fg: Color::Reset,
                muted: Color::DarkGray,
                border: Color::DarkGray,
                title: Color::White,
                accent: Color::Cyan,
                accent_fg: Color::Black,
                cpu: Color::Cyan,
                mem: Color::Magenta,
                swap: Color::Yellow,
                rx: Color::Green,
                tx: Color::Red,
                disk: Color::Yellow,
                gpu: Color::LightBlue,
                ok: Color::Green,
                warn: Color::Yellow,
                crit: Color::Red,
                selection_bg: Color::Blue,
                selection_fg: Color::White,
                track: Color::DarkGray,
            },
            ThemeName::Mono => Theme {
                name: "mono",
                fg: Color::Reset,
                muted: Color::DarkGray,
                border: Color::DarkGray,
                title: Color::White,
                accent: Color::White,
                accent_fg: Color::Black,
                cpu: Color::White,
                mem: Color::Gray,
                swap: Color::Gray,
                rx: Color::White,
                tx: Color::Gray,
                disk: Color::Gray,
                gpu: Color::White,
                ok: Color::Gray,
                warn: Color::White,
                crit: Color::White,
                selection_bg: Color::DarkGray,
                selection_fg: Color::White,
                track: Color::DarkGray,
            },
        }
    }

    /// Traffic-light color for a utilisation percentage.
    pub fn level(&self, pct: f64) -> Color {
        if pct >= 85.0 {
            self.crit
        } else if pct >= 60.0 {
            self.warn
        } else {
            self.ok
        }
    }
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_cycles_through_all_themes() {
        let mut t = ThemeName::Aurora;
        for _ in 0..ThemeName::ALL.len() {
            t = t.next();
        }
        assert_eq!(t, ThemeName::Aurora);
    }

    #[test]
    fn level_thresholds() {
        let t = Theme::get(ThemeName::Ansi);
        assert_eq!(t.level(10.0), t.ok);
        assert_eq!(t.level(70.0), t.warn);
        assert_eq!(t.level(95.0), t.crit);
    }
}
