/// A color theme. Light and dark variants of a family (e.g. Rosé Pine and
/// Rosé Pine Dawn) are separate themes, picked by the Dark Theme and Light
/// Theme settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeChoice {
    RosePine,
    Dracula,
    GruvboxDark,
    HamiMelonDark,
    AyuDark,
    RosePineDawn,
    /// Dracula's official light variant.
    Alucard,
    GruvboxLight,
    HamiMelonLight,
    AyuLight,
    CatppuccinMocha,
    CatppuccinLatte,
}

impl ThemeChoice {
    pub fn is_dark(self) -> bool {
        matches!(
            self,
            Self::RosePine
                | Self::Dracula
                | Self::GruvboxDark
                | Self::HamiMelonDark
                | Self::AyuDark
                | Self::CatppuccinMocha
        )
    }
}
