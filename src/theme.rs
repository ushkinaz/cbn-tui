use cursive::theme::{Color, PaletteColor, Theme};

/// Style for JSON highlighting
#[derive(Clone, Copy)]
pub struct JsonStyle {
    pub string: Color,
    pub number: Color,
    pub boolean: Color,
}

/// Returns a Cursive theme and JSON style based on the Solarized Dark color palette.
pub fn solarized_dark() -> (Theme, JsonStyle) {
    let mut theme = Theme::default();

    // Solarized Dark palette
    let base03 = Color::Rgb(0, 43, 54);
    let base02 = Color::Rgb(7, 54, 66);
    let base01 = Color::Rgb(88, 110, 117);
    let base00 = Color::Rgb(101, 123, 131);
    let base0 = Color::Rgb(131, 148, 150);
    let base3 = Color::Rgb(253, 246, 227);
    let yellow = Color::Rgb(181, 137, 0);
    let blue = Color::Rgb(38, 139, 210);

    let json_style = JsonStyle {
        string: Color::Rgb(133, 153, 0), // Green
        number: Color::Rgb(211, 54, 130), // Magenta
        boolean: Color::Rgb(220, 50, 47), // Red
    };

    {
        let palette = &mut theme.palette;

        palette[PaletteColor::Background] = base03;
        palette[PaletteColor::View] = base02;
        palette[PaletteColor::Shadow] = Color::Rgb(0, 0, 0);

        palette[PaletteColor::Primary] = base0;
        palette[PaletteColor::Secondary] = base01;
        palette[PaletteColor::Tertiary] = base00;

        palette[PaletteColor::TitlePrimary] = blue;
        palette[PaletteColor::TitleSecondary] = yellow;

        palette[PaletteColor::Highlight] = blue;
        palette[PaletteColor::HighlightInactive] = base01;
        palette[PaletteColor::HighlightText] = base3;
    }

    theme.borders = cursive::theme::BorderStyle::Simple;

    (theme, json_style)
}

/// Returns a Cursive theme and JSON style based on the Dracula color palette.
pub fn dracula_theme() -> (Theme, JsonStyle) {
    let mut theme = Theme::default();

    // Dracula palette
    let bg = Color::Rgb(40, 42, 54);
    let selection = Color::Rgb(68, 71, 90);
    let fg = Color::Rgb(248, 248, 242);
    let comment = Color::Rgb(98, 114, 164);
    let purple = Color::Rgb(189, 147, 249);
    let yellow = Color::Rgb(241, 250, 140);
    let orange = Color::Rgb(255, 184, 108);
    let pink = Color::Rgb(255, 121, 198);

    let json_style = JsonStyle {
        string: yellow,
        number: orange,
        boolean: pink,
    };

    {
        let palette = &mut theme.palette;

        palette[PaletteColor::Background] = bg;
        palette[PaletteColor::View] = bg;
        palette[PaletteColor::Shadow] = Color::Rgb(0, 0, 0);

        palette[PaletteColor::Primary] = fg;
        palette[PaletteColor::Secondary] = comment;
        palette[PaletteColor::Tertiary] = selection;

        palette[PaletteColor::TitlePrimary] = purple;
        palette[PaletteColor::TitleSecondary] = yellow;

        palette[PaletteColor::Highlight] = selection;
        palette[PaletteColor::HighlightInactive] = comment;
        palette[PaletteColor::HighlightText] = fg;
    }

    theme.borders = cursive::theme::BorderStyle::Simple;

    (theme, json_style)
}

/// Returns a Cursive theme and JSON style based on the Gruvbox Dark color palette.
pub fn gruvbox_theme() -> (Theme, JsonStyle) {
    let mut theme = Theme::default();

    // Gruvbox Dark palette
    let bg0 = Color::Rgb(40, 40, 40);
    let bg1 = Color::Rgb(50, 48, 47);
    let bg2 = Color::Rgb(60, 56, 54);
    let fg0 = Color::Rgb(251, 241, 199);
    let fg1 = Color::Rgb(235, 219, 178);
    let gray = Color::Rgb(146, 131, 116);
    let blue = Color::Rgb(69, 133, 136);
    let green = Color::Rgb(184, 187, 38);
    let orange = Color::Rgb(254, 128, 25);
    let purple = Color::Rgb(211, 134, 155);
    let yellow = Color::Rgb(250, 189, 47);

    let json_style = JsonStyle {
        string: green,
        number: purple,
        boolean: orange,
    };

    {
        let palette = &mut theme.palette;

        palette[PaletteColor::Background] = bg0;
        palette[PaletteColor::View] = bg1;
        palette[PaletteColor::Shadow] = Color::Rgb(0, 0, 0);

        palette[PaletteColor::Primary] = fg1;
        palette[PaletteColor::Secondary] = gray;
        palette[PaletteColor::Tertiary] = bg2;

        palette[PaletteColor::TitlePrimary] = orange;
        palette[PaletteColor::TitleSecondary] = yellow;

        palette[PaletteColor::Highlight] = blue;
        palette[PaletteColor::HighlightInactive] = bg2;
        palette[PaletteColor::HighlightText] = fg0;
    }

    theme.borders = cursive::theme::BorderStyle::Simple;

    (theme, json_style)
}

/// Returns a Cursive theme and JSON style based on the Everforest Light color palette.
pub fn everforest_light_theme() -> (Theme, JsonStyle) {
    let mut theme = Theme::default();

    // Everforest Light palette
    let bg = Color::Rgb(253, 246, 227);
    let bg_view = Color::Rgb(243, 234, 211);
    let fg = Color::Rgb(92, 106, 114);
    let gray = Color::Rgb(147, 159, 149);
    let yellow = Color::Rgb(223, 160, 0);
    let green = Color::Rgb(141, 161, 1);
    let red = Color::Rgb(230, 126, 128);
    let purple = Color::Rgb(214, 153, 182);

    let json_style = JsonStyle {
        string: green,
        number: red,
        boolean: purple,
    };

    {
        let palette = &mut theme.palette;

        palette[PaletteColor::Background] = bg;
        palette[PaletteColor::View] = bg_view;
        palette[PaletteColor::Shadow] = Color::Rgb(180, 180, 180);

        palette[PaletteColor::Primary] = fg;
        palette[PaletteColor::Secondary] = gray;
        palette[PaletteColor::Tertiary] = gray;

        palette[PaletteColor::TitlePrimary] = yellow;
        palette[PaletteColor::TitleSecondary] = green;

        palette[PaletteColor::Highlight] = gray;
        palette[PaletteColor::HighlightInactive] = bg_view;
        palette[PaletteColor::HighlightText] = bg;
    }

    theme.borders = cursive::theme::BorderStyle::Simple;

    (theme, json_style)
}
