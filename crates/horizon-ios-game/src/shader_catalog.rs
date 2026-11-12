//! Where a catalog page lists from: Horizon's registry or Modrinth. The installs are Starling's
//! own (starling-catalog).
use serde::{Deserialize, Serialize};
use starling_catalog::browser::SourceId;

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
