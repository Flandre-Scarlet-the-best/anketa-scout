use clap::ValueEnum;
use console::{Color, Style, Term};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    None,
    Ansi16,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug)]
pub struct Terminal {
    pub attended: bool,
    pub ansi: bool,
    pub no_color: bool,
    pub depth: ColorDepth,
    pub width: Option<u16>,
}

impl Terminal {
    pub fn detect(term: &Term) -> Self {
        let attended = term.features().is_attended();
        let term_name = std::env::var("TERM").unwrap_or_default();
        Self {
            attended,
            ansi: term_name != "dumb" && term.features().colors_supported(),
            no_color: std::env::var_os("NO_COLOR").is_some_and(|s| !s.is_empty()),
            depth: if std::env::var("COLORTERM").is_ok_and(|s| s == "truecolor" || s == "24bit")
                || (cfg!(windows) && std::env::var_os("WT_SESSION").is_some())
                || term.features().true_colors_supported()
            {
                ColorDepth::TrueColor
            } else if term_name.contains("256color") {
                ColorDepth::Ansi256
            } else {
                ColorDepth::Ansi16
            },
            // Размер берётся именно у выбранного потока; pipe не наследует размер stderr.
            width: attended
                .then(|| term.size_checked().map(|(_, width)| width))
                .flatten(),
        }
    }

    pub fn animation(self) -> bool {
        self.attended && self.ansi && self.width.is_some_and(|width| width >= 20)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Brand,
    Success,
    Contact,
    Post,
    Metadata,
    Heading,
    Text,
    Attachment,
    Border,
    Warning,
    Error,
    Loading,
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub theme: Theme,
    pub depth: ColorDepth,
}

impl Palette {
    pub fn resolve(mode: ColorMode, no_color: bool, theme: Theme, terminal: Terminal) -> Self {
        let enabled = !no_color
            && match mode {
                ColorMode::Never => false,
                ColorMode::Always => true,
                ColorMode::Auto => terminal.attended && terminal.ansi && !terminal.no_color,
            };
        Self {
            theme,
            depth: if enabled {
                terminal.depth
            } else {
                ColorDepth::None
            },
        }
    }

    pub fn paint(self, role: Role, text: &str) -> String {
        if self.depth == ColorDepth::None {
            return text.to_owned();
        }
        let light = matches!(self.theme, Theme::Light);
        let (dark_rgb, light_rgb, dark256, light256, base) = match role {
            Role::Brand => ((196, 167, 255), (88, 40, 150), 183, 54, Color::Magenta),
            Role::Success => ((139, 213, 160), (26, 105, 54), 114, 22, Color::Green),
            Role::Contact => ((114, 214, 208), (0, 100, 105), 116, 23, Color::Cyan),
            Role::Post => ((196, 167, 255), (88, 40, 150), 183, 54, Color::Magenta),
            Role::Metadata | Role::Heading => {
                ((152, 162, 179), (75, 85, 101), 145, 240, Color::White)
            }
            Role::Text => ((230, 233, 240), (31, 41, 55), 255, 235, Color::White),
            Role::Attachment => ((152, 182, 211), (38, 83, 132), 110, 24, Color::Blue),
            Role::Border => ((89, 101, 121), (112, 123, 139), 60, 243, Color::White),
            Role::Warning => ((240, 198, 116), (132, 83, 0), 222, 94, Color::Yellow),
            Role::Error => ((242, 139, 152), (170, 30, 57), 210, 125, Color::Red),
            Role::Loading => ((152, 162, 179), (75, 85, 101), 145, 240, Color::Cyan),
        };
        let mut style = match self.depth {
            ColorDepth::TrueColor => {
                let (r, g, b) = if light { light_rgb } else { dark_rgb };
                Style::new().true_color(r, g, b)
            }
            ColorDepth::Ansi256 => Style::new().color256(if light { light256 } else { dark256 }),
            ColorDepth::Ansi16 => {
                let base = if light
                    && matches!(
                        role,
                        Role::Text | Role::Metadata | Role::Heading | Role::Border
                    ) {
                    Color::Black
                } else {
                    base
                };
                let style = Style::new().fg(base);
                if !light && matches!(role, Role::Metadata | Role::Border) {
                    style.dim()
                } else {
                    style
                }
            }
            ColorDepth::None => unreachable!(),
        };
        if matches!(
            role,
            Role::Brand | Role::Heading | Role::Success | Role::Contact | Role::Post | Role::Error
        ) {
            style = style.bold();
        }
        style.force_styling(true).apply_to(text).to_string()
    }
}
