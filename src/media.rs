#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MediaPreviewProtocol {
    #[default]
    Auto,
    Halfblocks,
    Kitty,
    Iterm2,
    Sixel,
}

impl MediaPreviewProtocol {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Halfblocks => "halfblocks",
            Self::Kitty => "kitty",
            Self::Iterm2 => "iterm2",
            Self::Sixel => "sixel",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::Auto => Self::Halfblocks,
            Self::Halfblocks => Self::Kitty,
            Self::Kitty => Self::Iterm2,
            Self::Iterm2 => Self::Sixel,
            Self::Sixel => Self::Auto,
        }
    }
}
