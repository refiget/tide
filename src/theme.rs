use crossterm::style::Color;

pub struct CatppuccinFrappe;

impl CatppuccinFrappe {
    pub const LAVENDER: Color = Color::Rgb {
        r: 186,
        g: 187,
        b: 241,
    };
    pub const TEXT: Color = Color::Rgb {
        r: 198,
        g: 208,
        b: 245,
    };
    pub const SUBTEXT1: Color = Color::Rgb {
        r: 181,
        g: 191,
        b: 226,
    };
    pub const SURFACE2: Color = Color::Rgb {
        r: 98,
        g: 104,
        b: 128,
    };
    pub const SURFACE1: Color = Color::Rgb {
        r: 81,
        g: 87,
        b: 109,
    };
    pub const SURFACE0: Color = Color::Rgb {
        r: 65,
        g: 69,
        b: 89,
    };
    pub const MANTLE: Color = Color::Rgb {
        r: 41,
        g: 44,
        b: 60,
    };
    pub const GREEN: Color = Color::Rgb {
        r: 166,
        g: 209,
        b: 137,
    };
    pub const RED: Color = Color::Rgb {
        r: 231,
        g: 130,
        b: 132,
    };
    pub const YELLOW: Color = Color::Rgb {
        r: 229,
        g: 200,
        b: 144,
    };
    pub const SUBTEXT0: Color = Color::Rgb {
        r: 165,
        g: 173,
        b: 206,
    };
    pub const MAUVE: Color = Color::Rgb {
        r: 202,
        g: 158,
        b: 230,
    };
    pub const BLUE: Color = Color::Rgb {
        r: 140,
        g: 170,
        b: 238,
    };
    pub const TEAL: Color = Color::Rgb {
        r: 129,
        g: 200,
        b: 190,
    };
}

pub struct Theme;

impl Theme {
    pub const BORDER_NORMAL_FG: Color = CatppuccinFrappe::SURFACE2;
    pub const BORDER_SELECTED_FG: Color = CatppuccinFrappe::LAVENDER;
    pub const BODY_SELECTED_BG: Color = CatppuccinFrappe::SURFACE0;
    pub const BODY_SELECTED_FG: Color = CatppuccinFrappe::TEXT;
    pub const CURSOR_BG: Color = CatppuccinFrappe::SURFACE1;
    pub const CURSOR_FG: Color = CatppuccinFrappe::TEXT;
    pub const FOOTER_BG: Color = CatppuccinFrappe::MANTLE;
    pub const FOOTER_FG: Color = CatppuccinFrappe::SUBTEXT1;
    pub const FOOTER_KEY_FG: Color = CatppuccinFrappe::LAVENDER;
    pub const FOOTER_SEP_FG: Color = CatppuccinFrappe::SURFACE2;
    pub const DETAIL_BORDER_FG: Color = CatppuccinFrappe::LAVENDER;
    pub const STATUS_OK_FG: Color = CatppuccinFrappe::GREEN;
    pub const STATUS_FAILED_FG: Color = CatppuccinFrappe::RED;
    pub const STATUS_RUNNING_FG: Color = CatppuccinFrappe::YELLOW;
    pub const META_LABEL_FG: Color = CatppuccinFrappe::SUBTEXT0;
    pub const META_HEADER_FG: Color = CatppuccinFrappe::MAUVE;
    pub const META_PATH_FG: Color = CatppuccinFrappe::BLUE;
    pub const META_ACTION_KEY_FG: Color = CatppuccinFrappe::MAUVE;
    pub const META_ACTION_TEXT_FG: Color = CatppuccinFrappe::BLUE;
    pub const HELP_BG: Color = CatppuccinFrappe::SURFACE0;
    pub const HELP_BORDER: Color = CatppuccinFrappe::MAUVE;
    pub const HELP_KEY_FG: Color = CatppuccinFrappe::TEAL;
    pub const HELP_TEXT_FG: Color = CatppuccinFrappe::SUBTEXT1;
    pub const HELP_SEL_BG: Color = CatppuccinFrappe::SURFACE1;
    pub const HELP_SEL_FG: Color = CatppuccinFrappe::TEXT;
    pub const HELP_DIM_FG: Color = CatppuccinFrappe::SUBTEXT0;
    pub const SEARCH_MATCH_FG: Color = CatppuccinFrappe::YELLOW;
    pub const VISUAL_BORDER_FG: Color = CatppuccinFrappe::YELLOW;
    pub const VISUAL_LINE_BG: Color = CatppuccinFrappe::SURFACE1;
    // Nerd Font icon colors — each role gets a distinct Frappe hue.
    // Status icons (ok/fail/running) use STATUS_* colors for semantic meaning.
    pub const ICON_SECTION_FG: Color = CatppuccinFrappe::MAUVE;    // 󰋼  section headers
    pub const ICON_CMD_FG: Color = CatppuccinFrappe::BLUE;          // 󰘧  command · 󰆏 copy-cmd
    pub const ICON_PATH_FG: Color = CatppuccinFrappe::TEAL;         // 󰉋  cwd · 󰉆 copy-out
    pub const ICON_TIME_FG: Color = CatppuccinFrappe::YELLOW;       // 󰔟  duration
    pub const ICON_ACTION_FG: Color = CatppuccinFrappe::LAVENDER;   // 󰘳  actions · 󰈚 y · 󰑓 rerun
}
