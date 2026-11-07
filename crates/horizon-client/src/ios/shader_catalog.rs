//! Shader/mod source selection for iOS.
use crate::browser::SourceId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    #[default]
    Horizon,
    Modrinth,
}
impl Origin {
    pub fn source(self) -> SourceId {
        match self {
            Self::Horizon => SourceId::Horizon,
            Self::Modrinth => SourceId::Modrinth,
        }
    }
}
