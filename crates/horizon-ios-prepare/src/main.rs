//! `horizon-ios-prepare`, CI's half of Starling's game files on iOS (lib.rs):
//!
//!     horizon-ios-prepare pack --resources DIR --minecraft DIR --version V --out ZIP
//!         horizon-prepare's folder split over the official files: the zip the app carries
//!     horizon-ios-prepare check --engine ZIP --minecraft DIR --reference DIR [--work DIR]
//!         assemble from the zip and the official files as the phone does, and fail unless the
//!         result is the reference file for file, and the zip holds no Mojang file
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("usage: horizon-ios-prepare pack|check ... (see the source's first lines)");
        return ExitCode::from(2);
    };
    let mut options = std::collections::HashMap::new();
    while let Some(flag) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("{flag} needs a value");
            return ExitCode::from(2);
        };
        options.insert(flag, value);
    }
    let path = |name: &str| options.get(name).map(PathBuf::from);
    let result = match command.as_str() {
        "pack" => (|| {
            let (Some(resources), Some(minecraft), Some(out)) =
                (path("--resources"), path("--minecraft"), path("--out"))
            else {
                return Err("pack needs --resources, --minecraft and --out".to_owned());
            };
            let version = options.get("--version").ok_or("pack needs --version")?;
            let packed = horizon_ios_prepare::pack(&resources, &minecraft, version, &out)?;
            println!(
                "{}: {} generated files kept, {} from the client jar and {} asset objects left \
                 to the player's own download",
                out.display(),
                packed.generated,
                packed.from_jar,
                packed.from_objects
            );
            Ok(())
        })(),
        "check" => (|| {
            let (Some(engine), Some(minecraft), Some(reference)) =
                (path("--engine"), path("--minecraft"), path("--reference"))
            else {
                return Err("check needs --engine, --minecraft and --reference".to_owned());
            };
            let work =
                path("--work").unwrap_or_else(|| std::env::temp_dir().join("ios-assembled"));
            let version = options.get("--version").ok_or("check needs --version")?;
            let foreign = horizon_ios_prepare::foreign(&engine, &minecraft, version)?;
            if !foreign.is_empty() {
                return Err(format!(
                    "the app's tables carry Mojang files: {}",
                    foreign[..foreign.len().min(10)].join(", ")
                ));
            }
            horizon_ios_prepare::assemble(&engine, &minecraft, &work, &|done, total| {
                if done == total || done % 5000 == 0 {
                    println!("assembled {done} of {total} files");
                }
            })?;
            let differences = horizon_ios_prepare::compare(&reference, &work, 50)?;
            if !differences.is_empty() {
                return Err(format!(
                    "the phone's folder would differ from horizon-prepare's:\n  {}",
                    differences.join("\n  ")
                ));
            }
            println!("{} is horizon-prepare's folder, file for file", work.display());
            Ok(())
        })(),
        other => Err(format!("unknown command {other}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("horizon-ios-prepare: {e}");
            ExitCode::FAILURE
        }
    }
}
