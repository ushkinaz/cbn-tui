use ratatui::style::{Color, Modifier, Style};

/// Style for JSON highlighting
#[derive(Clone, Copy)]
pub struct JsonStyle {
    pub key: Color,
    pub string: Color,
    pub number: Color,
    pub boolean: Color,
}

/// Complete theme configuration for ratatui
#[derive(Clone)]
pub struct ThemeConfig {
    #[allow(dead_code)]
    pub background: Color,
    pub list_normal: Style,
    pub list_selected: Style,
    pub border: Style,
    pub border_selected: Style,
    pub title: Style,
    pub text: Style,
    pub json_style: JsonStyle,
}

/// Returns a ThemeConfig based on the Solarized Dark color palette.
#[allow(unused_variables)]
pub fn solarized_dark() -> ThemeConfig {
    // Solarized Dark palette
    let base03 = Color::Rgb(0, 43, 54);
    let base02 = Color::Rgb(7, 54, 66);
    let base01 = Color::Rgb(88, 110, 117);
    let base0 = Color::Rgb(131, 148, 150);
    let base3 = Color::Rgb(253, 246, 227);
    let yellow = Color::Rgb(181, 137, 0);
    let orange = Color::Rgb(203, 75, 22);
    let red = Color::Rgb(220, 50, 47);
    let magenta = Color::Rgb(211, 54, 130);
    let blue = Color::Rgb(38, 139, 210);
    let cyan = Color::Rgb(42, 161, 152);
    let green = Color::Rgb(133, 153, 0);

    let json_style = JsonStyle {
        key: cyan,
        string: green,
        number: magenta,
        boolean: red,
    };

    ThemeConfig {
        background: base03,
        list_normal: Style::default().fg(base0).bg(base02),
        list_selected: Style::default()
            .fg(base3)
            .bg(blue)
            .add_modifier(Modifier::BOLD),
        border: Style::default().fg(base01),
        border_selected: Style::default().fg(blue),
        title: Style::default().fg(blue).add_modifier(Modifier::BOLD),
        text: Style::default().fg(base0).bg(base02),
        json_style,
    }
}

/// Returns a ThemeConfig based on the Dracula color palette.
#[allow(unused_variables)]
pub fn dracula_theme() -> ThemeConfig {
    // Dracula palette
    let bg = Color::Rgb(40, 42, 54);
    let selection = Color::Rgb(68, 71, 90);
    let fg = Color::Rgb(248, 248, 242);
    let comment = Color::Rgb(98, 114, 164);
    let purple = Color::Rgb(189, 147, 249);
    let yellow = Color::Rgb(241, 250, 140);
    let orange = Color::Rgb(255, 184, 108);
    let pink = Color::Rgb(255, 121, 198);
    let cyan = Color::Rgb(139, 233, 253);

    let json_style = JsonStyle {
        key: cyan,
        string: yellow,
        number: orange,
        boolean: pink,
    };

    ThemeConfig {
        background: bg,
        list_normal: Style::default().fg(fg).bg(bg),
        list_selected: Style::default()
            .fg(fg)
            .bg(selection)
            .add_modifier(Modifier::BOLD),
        border: Style::default().fg(comment),
        border_selected: Style::default().fg(purple),
        title: Style::default().fg(purple).add_modifier(Modifier::BOLD),
        text: Style::default().fg(fg).bg(bg),
        json_style,
    }
}

/// Returns a ThemeConfig based on the Gruvbox Dark color palette.
#[allow(unused_variables)]
pub fn gruvbox_theme() -> ThemeConfig {
    // Gruvbox Dark palette
    let bg0 = Color::Rgb(40, 40, 40);
    let bg1 = Color::Rgb(60, 56, 54); // bg2
    let fg0 = Color::Rgb(251, 241, 199);
    let fg1 = Color::Rgb(235, 219, 178);
    let gray = Color::Rgb(146, 131, 116);
    let blue = Color::Rgb(69, 133, 136);
    let green = Color::Rgb(152, 151, 26);
    let orange = Color::Rgb(214, 93, 14);
    let purple = Color::Rgb(177, 98, 134);

    let json_style = JsonStyle {
        key: blue,
        string: green,
        number: purple,
        boolean: orange,
    };

    ThemeConfig {
        background: bg0,
        list_normal: Style::default().fg(fg1).bg(bg0),
        list_selected: Style::default()
            .fg(bg0)
            .bg(fg1)
            .add_modifier(Modifier::BOLD),
        border: Style::default().fg(gray),
        border_selected: Style::default().fg(orange),
        title: Style::default().fg(orange).add_modifier(Modifier::BOLD),
        text: Style::default().fg(fg1).bg(bg0),
        json_style,
    }
}

/// Returns a ThemeConfig based on the Everforest Light color palette.
#[allow(unused_variables)]
pub fn everforest_light_theme() -> ThemeConfig {
    // Everforest Light palette
    let bg = Color::Rgb(253, 246, 227);
    let bg_view = Color::Rgb(243, 234, 211);
    let fg = Color::Rgb(92, 106, 114);
    let gray = Color::Rgb(147, 159, 149);
    let yellow = Color::Rgb(223, 160, 0);
    let green = Color::Rgb(141, 161, 1);
    let red = Color::Rgb(248, 85, 82);
    let blue = Color::Rgb(58, 148, 197);
    let magenta = Color::Rgb(223, 105, 186);

    let json_style = JsonStyle {
        key: blue,
        string: green,
        number: red,
        boolean: magenta,
    };

    ThemeConfig {
        background: bg,
        list_normal: Style::default().fg(fg).bg(bg_view),
        list_selected: Style::default()
            .fg(bg)
            .bg(gray)
            .add_modifier(Modifier::BOLD),
        border: Style::default().fg(gray),
        border_selected: Style::default().fg(yellow),
        title: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
        text: Style::default().fg(fg).bg(bg_view),
        json_style,
    }
}
