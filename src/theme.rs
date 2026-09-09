use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub canvas: Color,
    pub panel: Color,
    pub surface_alt: Color,
    pub raised: Color,
    pub selected: Color,
    pub inactive_selected: Color,
    pub ink: Color,
    pub soft: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub purple: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub cyan: Color,
    pub orange: Color,
    pub add_bg: Color,
    pub add_word_bg: Color,
    pub remove_bg: Color,
    pub remove_word_bg: Color,
    pub graph_colors: [Color; 8],
}

impl Default for Palette {
    fn default() -> Self {
        load_theme()
    }
}

pub fn load_theme() -> Palette {
    let accent = rgb(0x8a, 0xad, 0xf4);
    Palette {
        canvas: rgb(0x24, 0x27, 0x3a),
        panel: rgb(0x1e, 0x20, 0x30),
        surface_alt: rgb(0x18, 0x19, 0x26),
        raised: rgb(0x36, 0x3a, 0x4f),
        selected: rgb(0x49, 0x4d, 0x64),
        inactive_selected: rgb(0x36, 0x3a, 0x4f),
        ink: rgb(0xca, 0xd3, 0xf5),
        soft: rgb(0xae, 0xb6, 0xd6),
        muted: rgb(0x93, 0x9a, 0xb7),
        faint: rgb(0x5b, 0x60, 0x78),
        accent,
        purple: rgb(0xc6, 0xa0, 0xf6),
        green: rgb(0xa6, 0xda, 0x95),
        yellow: rgb(0xee, 0xd4, 0x9f),
        red: rgb(0xed, 0x87, 0x96),
        cyan: rgb(0x8b, 0xd5, 0xca),
        orange: rgb(0xf5, 0xa9, 0x7f),
        add_bg: rgb(0x29, 0x34, 0x2b),
        add_word_bg: rgb(0x34, 0x4f, 0x38),
        remove_bg: rgb(0x3a, 0x2a, 0x31),
        remove_word_bg: rgb(0x54, 0x32, 0x3c),
        graph_colors: [
            accent,
            rgb(0xc6, 0xa0, 0xf6),
            rgb(0xa6, 0xda, 0x95),
            rgb(0xee, 0xd4, 0x9f),
            rgb(0xed, 0x87, 0x96),
            rgb(0x8b, 0xd5, 0xca),
            rgb(0xf5, 0xa9, 0x7f),
            rgb(0xf5, 0xbd, 0xe6),
        ],
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgb(red, green, blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_is_stable() {
        let palette = load_theme();
        assert_eq!(palette.canvas, Color::Rgb(0x24, 0x27, 0x3a));
        assert_eq!(palette.accent, Color::Rgb(0x8a, 0xad, 0xf4));
        assert_eq!(palette.add_bg, Color::Rgb(0x29, 0x34, 0x2b));
    }
}
