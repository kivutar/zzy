#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    RosePine,
    Dracula,
    GithubDark,
    GruvboxDark,
    HamiMelonDark,
    AyuDark,
}

impl ThemeChoice {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeChoice::RosePine => "rose-pine",
            ThemeChoice::Dracula => "dracula",
            ThemeChoice::GithubDark => "github-dark",
            ThemeChoice::GruvboxDark => "gruvbox-dark",
            ThemeChoice::HamiMelonDark => "hami-melon-dark",
            ThemeChoice::AyuDark => "ayu-dark",
        }
    }
}
