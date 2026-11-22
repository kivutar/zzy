#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeChoice {
    RosePine,
    Dracula,
    GruvboxDark,
    HamiMelonDark,
    AyuDark,
}
