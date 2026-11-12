//! Horizon Android's installer, as its launcher calls it (stages, progress, ready), over Starling's
//! own: the official files Starling's setup fetches and checks (horizon_client::accounts) and the
//! iOS preparing that lays them out with the tables CI made (horizon_client::preparer,
//! horizon-ios-prepare). The game folder's resources are Starling's private `resources`.
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstallStage {
    #[default]
    Metadata,
    Download,
    Extract,
    WorldData,
    Controls,
    Finalize,
}

impl InstallStage {
    pub const ALL: [Self; 6] = [
        Self::Metadata,
        Self::Download,
        Self::Extract,
        Self::WorldData,
        Self::Controls,
        Self::Finalize,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata => "Checking Minecraft version",
            Self::Download => "Downloading game resources and sound",
            Self::Extract => "Unpacking verified files",
            Self::WorldData => "Preparing worlds and registries",
            Self::Controls => "Downloading touch controls",
            Self::Finalize => "Finishing installation",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InstallProgress {
    pub stage: InstallStage,
    pub done: u64,
    pub total: u64,
}

/// Where the game's resources are: Starling's private folder's `resources`.
pub fn resources() -> PathBuf {
    horizon_client::logging::private_home().join("resources")
}

/// The game tables the app carries (tools/ios_prepare.py, packaging/ios/project.yml).
fn engine() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("assets/engine-data.zip"))
}

/// Whether the resources are laid out, from the tables this app carries (an update with new
/// tables installs again).
pub fn ready(_home: &Path) -> bool {
    let Some(engine) = engine() else {
        return false;
    };
    match (
        horizon_ios_prepare::engine_id(&engine),
        horizon_ios_prepare::assembled_id(&resources()),
    ) {
        (Some(bundled), Some(laid_out)) => bundled == laid_out,
        _ => false,
    }
}

/// Fetch the official files (each checked), then lay the game out, reporting each stage.
pub fn install_with_progress(
    _home: &Path,
    progress: impl Fn(InstallProgress) + Sync,
) -> Result<(), String> {
    use horizon_client::accounts::{self, Files};
    use horizon_client::preparer::{self, Stage};
    let report = |stage, done, total| {
        progress(InstallProgress { stage, done, total });
    };
    report(InstallStage::Metadata, 0, 0);
    accounts::download_files();
    loop {
        match accounts::files() {
            Files::Ready => break,
            Files::Failed(e) => return Err(e),
            Files::Missing => return Err("The download stopped. Choose Download again.".into()),
            Files::Downloading(p) => report(InstallStage::Download, p.bytes, p.bytes_total),
            Files::Checking => report(InstallStage::Metadata, 0, 0),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    report(InstallStage::Extract, 0, 0);
    preparer::start(resources());
    loop {
        match preparer::stage() {
            Stage::Done => {
                preparer::clear();
                break;
            }
            Stage::Failed(e) => return Err(e),
            Stage::Preparing(line) => {
                let (done, total) = counts(&line);
                report(InstallStage::WorldData, done, total);
            }
            Stage::Idle | Stage::Java(..) | Stage::Unpacking => report(InstallStage::Extract, 0, 0),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    report(InstallStage::Finalize, 1, 1);
    if !ready(Path::new("")) {
        return Err("The game files were laid out, but not from this app's tables.".into());
    }
    Ok(())
}

/// "1234 of 5678 files" as (1234, 5678).
fn counts(line: &str) -> (u64, u64) {
    let mut words = line.split_whitespace();
    let done = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
    let total = words.nth(1).and_then(|w| w.parse().ok()).unwrap_or(0);
    (done, total)
}

#[cfg(test)]
mod tests {
    #[test]
    fn preparing_lines_give_their_counts() {
        assert_eq!(super::counts("1234 of 5678 files"), (1234, 5678));
        assert_eq!(super::counts("Starting"), (0, 0));
    }
}
