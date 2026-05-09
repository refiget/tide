use crossterm::style::Color;

pub struct CatppuccinFrappe;

impl CatppuccinFrappe {
    pub const LAVENDER: Color = Color::Rgb { r: 186, g: 187, b: 241 };
    pub const TEXT: Color     = Color::Rgb { r: 198, g: 208, b: 245 };
    pub const SUBTEXT1: Color = Color::Rgb { r: 181, g: 191, b: 226 };
    pub const SURFACE2: Color = Color::Rgb { r: 98,  g: 104, b: 128 };
    pub const SURFACE1: Color = Color::Rgb { r: 81,  g: 87,  b: 109 };
    pub const SURFACE0: Color = Color::Rgb { r: 65,  g: 69,  b: 89  };
    pub const MANTLE: Color   = Color::Rgb { r: 41,  g: 44,  b: 60  };
}

pub struct Theme;

impl Theme {
    pub const BORDER_NORMAL_FG: Color   = CatppuccinFrappe::SURFACE2;
    pub const BORDER_SELECTED_FG: Color = CatppuccinFrappe::LAVENDER;
    pub const BODY_SELECTED_BG: Color   = CatppuccinFrappe::SURFACE0;
    pub const BODY_SELECTED_FG: Color   = CatppuccinFrappe::TEXT;
    pub const CURSOR_BG: Color          = CatppuccinFrappe::SURFACE1;
    pub const CURSOR_FG: Color          = CatppuccinFrappe::TEXT;
    pub const FOOTER_BG: Color          = CatppuccinFrappe::MANTLE;
    pub const FOOTER_FG: Color          = CatppuccinFrappe::SUBTEXT1;
    pub const DETAIL_BORDER_FG: Color   = CatppuccinFrappe::LAVENDER;
}
