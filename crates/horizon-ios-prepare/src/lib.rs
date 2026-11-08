//! Starling's game files on iOS, where no Java runs: Starling's Android app prepares them on the
//! phone with horizon-prepare and a Java runtime it downloads; iOS lets an app run neither.
//!
//! So CI runs that same horizon-prepare once, over the pinned official files, and [`pack`] splits
//! what it made in two: the tables horizon-prepare generated (the exporters' output, its own
//! tables, the manifest), which go in the app as `engine-data.zip`, and the files that are
//! Mojang's own (the client jar's assets and data, the asset objects: textures, sounds,
//! languages), which never do. For each of those the zip holds a recipe line: which jar member or
//! asset object it is, and its SHA-256.
//!
//! On the phone, once the setup has fetched the player's own official files (accounts.rs,
//! checked against research-lock.json), [`assemble`] lays the folder out again from the two:
//! the same files, byte for byte, as horizon-prepare made in CI. CI proves it every build:
//! `horizon-ios-prepare check` assembles from the zip and the downloads and [`compare`]s the result
//! with horizon-prepare's own folder, file by file.
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

/// The recipe inside the zip.
pub const RECIPE: &str = "ios-recipe.json";
/// What the recipe's format is: bump when it changes.
const FORMAT: u64 = 1;
/// Where Mojang serves asset objects by their hash.
const OBJECTS: &str = "https://resources.download.minecraft.net";

/// Files of a preparation the game never reads: horizon-prepare's work (its logs, the exporters'
/// compiled classes, its lock), and the caches the game writes there itself at run time.
pub fn work_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/')
        .any(|part| part == "classes" || part == "client-classes")
        || name.ends_with(".log")
        || name.ends_with(".class")
        || matches!(
            name,
            ".horizon-prepare.lock" | "horizon-data.bin" | "horizon-sprites.cache"
        )
}

/// What [`pack`] made.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Packed {
    pub generated: usize,
    pub from_jar: usize,
    pub from_objects: usize,
}

fn sha256_of(mut reader: impl Read) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    File::open(path)
        .and_then(sha256_of)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Every file under `root` but the work files, by its `/`-separated path, in order.
pub fn files(root: &Path) -> Result<Vec<String>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
        let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                if !work_file(&relative) {
                    out.push(relative);
                }
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

/// A recipe or zip path, which must stay inside the folder it is laid out in.
fn inside(path: &str) -> Result<&Path, String> {
    let relative = Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("Unsafe path in the game files: {path}"));
    }
    Ok(relative)
}

/// The pinned version's client jar in the official launcher's layout.
fn client_jar(minecraft: &Path, version: &str) -> PathBuf {
    minecraft
        .join("versions")
        .join(version)
        .join(format!("{version}.jar"))
}

fn object_path(minecraft: &Path, sha1: &str) -> PathBuf {
    minecraft.join("assets/objects").join(&sha1[..2]).join(sha1)
}

/// `resources` (horizon-prepare's folder for `version`) split over the official files in
/// `minecraft` (the launcher layout): the generated files and the recipe, into the zip `out`.
pub fn pack(
    resources: &Path,
    minecraft: &Path,
    version: &str,
    out: &Path,
) -> Result<Packed, String> {
    // The client jar's members and the asset objects, by their SHA-256.
    let jar = client_jar(minecraft, version);
    let mut archive =
        zip::ZipArchive::new(File::open(&jar).map_err(|e| format!("{}: {e}", jar.display()))?)
            .map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut members: HashMap<String, Vec<String>> = HashMap::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        let sha = sha256_of(entry).map_err(|e| format!("{name}: {e}"))?;
        members.entry(sha).or_default().push(name);
    }
    let mut objects: HashMap<String, String> = HashMap::new();
    let objects_dir = minecraft.join("assets/objects");
    if let Ok(prefixes) = fs::read_dir(&objects_dir) {
        for prefix in prefixes.flatten() {
            for object in fs::read_dir(prefix.path()).into_iter().flatten().flatten() {
                let sha1 = object.file_name().to_string_lossy().into_owned();
                if sha1.len() == 40 && sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
                    objects.insert(sha256_file(&object.path())?, sha1);
                }
            }
        }
    }
    let mut recipe = Vec::new();
    let mut packed = Packed::default();
    let mut zip =
        zip::ZipWriter::new(File::create(out).map_err(|e| format!("{}: {e}", out.display()))?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for path in files(resources)? {
        let file = resources.join(inside(&path)?);
        let sha = sha256_file(&file)?;
        let line = if let Some(names) = members.get(&sha) {
            // The member of the same name, when the jar has it, else any with these bytes.
            let entry = names.iter().find(|n| **n == path).unwrap_or(&names[0]);
            packed.from_jar += 1;
            serde_json::json!({ "path": path, "jar": entry, "sha256": sha })
        } else if let Some(sha1) = objects.get(&sha) {
            packed.from_objects += 1;
            serde_json::json!({ "path": path, "object": sha1, "sha256": sha })
        } else {
            packed.generated += 1;
            zip.start_file(path.as_str(), options)
                .map_err(|e| e.to_string())?;
            io::copy(
                &mut File::open(&file).map_err(|e| format!("{path}: {e}"))?,
                &mut zip,
            )
            .map_err(|e| format!("{path}: {e}"))?;
            serde_json::json!({ "path": path, "sha256": sha })
        };
        recipe.push(line);
    }
    let recipe = serde_json::json!({
        "format": FORMAT,
        "version": version,
        "files": recipe,
    });
    zip.start_file(RECIPE, options).map_err(|e| e.to_string())?;
    zip.write_all(recipe.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(packed)
}

/// One line of the recipe.
enum Source {
    Zip,
    Jar(String),
    Object(String),
}

/// Copies `input` to `path`, checking its SHA-256 on the way.
fn write_checked(mut input: impl Read, path: &Path, sha: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut output = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let n = input
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
        output
            .write_all(&buffer[..n])
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if format!("{:x}", digest.finalize()) != sha {
        return Err(format!("{} is not the file it should be", path.display()));
    }
    Ok(())
}

/// An asset object the download lacks (or holds damaged), fetched by its hash from Mojang.
fn fetch_object(minecraft: &Path, sha1: &str) -> Result<PathBuf, String> {
    use sha1::Sha1;
    let path = object_path(minecraft, sha1);
    let url = format!("{OBJECTS}/{}/{sha1}", &sha1[..2]);
    let mut bytes = Vec::new();
    ureq::get(&url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_reader()
        .take(1 << 30)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{url}: {e}"))?;
    if format!("{:x}", Sha1::digest(&bytes)) != sha1 {
        return Err(format!("{url} is not the file it should be"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The prepared folder at `out` again: the generated files from `engine` (the zip [`pack`] made)
/// and Mojang's own from the official files in `minecraft`. Laid out beside `out` first and moved
/// in once complete, so a stop midway leaves the last complete folder (or none). `progress` hears
/// the files laid out so far and in all.
pub fn assemble(
    engine: &Path,
    minecraft: &Path,
    out: &Path,
    progress: &dyn Fn(usize, usize),
) -> Result<(), String> {
    let mut engine =
        zip::ZipArchive::new(File::open(engine).map_err(|e| format!("{}: {e}", engine.display()))?)
            .map_err(|e| format!("The app's game tables: {e}"))?;
    let recipe: serde_json::Value = {
        let entry = engine
            .by_name(RECIPE)
            .map_err(|e| format!("The app's game tables have no recipe: {e}"))?;
        serde_json::from_reader(entry).map_err(|e| format!("The app's recipe: {e}"))?
    };
    if recipe["format"].as_u64() != Some(FORMAT) {
        return Err("The app's recipe is of another format".into());
    }
    let version = recipe["version"]
        .as_str()
        .ok_or("The recipe names no version")?;
    let lines = recipe["files"]
        .as_array()
        .ok_or("The recipe lists no files")?;
    let mut plan = Vec::with_capacity(lines.len());
    for line in lines {
        let path = line["path"].as_str().ok_or("A recipe line has no path")?;
        inside(path)?;
        let sha = line["sha256"].as_str().ok_or("A recipe line has no hash")?;
        let source = if let Some(entry) = line["jar"].as_str() {
            Source::Jar(entry.to_owned())
        } else if let Some(sha1) = line["object"].as_str() {
            if sha1.len() != 40 || !sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("A recipe line names a bad object: {sha1}"));
            }
            Source::Object(sha1.to_owned())
        } else {
            Source::Zip
        };
        plan.push((path.to_owned(), sha.to_owned(), source));
    }
    let stage = out.with_extension("partial");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    }
    fs::create_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    let jar_path = client_jar(minecraft, version);
    let mut jar = None;
    let total = plan.len();
    for (done, (path, sha, source)) in plan.iter().enumerate() {
        let target = stage.join(inside(path)?);
        match source {
            Source::Zip => {
                let entry = engine
                    .by_name(path)
                    .map_err(|e| format!("The app's game tables lack {path}: {e}"))?;
                write_checked(entry, &target, sha)?;
            }
            Source::Jar(name) => {
                if jar.is_none() {
                    let file = File::open(&jar_path)
                        .map_err(|e| format!("{}: {e}", jar_path.display()))?;
                    jar = Some(
                        zip::ZipArchive::new(file)
                            .map_err(|e| format!("{}: {e}", jar_path.display()))?,
                    );
                }
                let Some(jar) = jar.as_mut() else {
                    return Err("The client jar did not open".into());
                };
                let entry = jar
                    .by_name(name)
                    .map_err(|e| format!("The client jar lacks {name}: {e}"))?;
                write_checked(entry, &target, sha)?;
            }
            Source::Object(sha1) => {
                let mut from = object_path(minecraft, sha1);
                let present = File::open(&from).and_then(sha256_of).ok();
                if present.as_deref() != Some(sha.as_str()) {
                    from = fetch_object(minecraft, sha1)?;
                }
                let input = File::open(&from).map_err(|e| format!("{}: {e}", from.display()))?;
                write_checked(input, &target, sha)?;
            }
        }
        progress(done + 1, total);
    }
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    fs::rename(&stage, out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(())
}

/// Whether `built` holds what `reference` does, file for file and byte for byte (the work files
/// aside): the differences, at most `limit` of them, when it does not.
pub fn compare(reference: &Path, built: &Path, limit: usize) -> Result<Vec<String>, String> {
    let hashes = |root: &Path| -> Result<BTreeMap<String, String>, String> {
        files(root)?
            .into_iter()
            .map(|path| {
                let sha = sha256_file(&root.join(&path))?;
                Ok((path, sha))
            })
            .collect()
    };
    let (want, have) = (hashes(reference)?, hashes(built)?);
    let mut differences = Vec::new();
    for (path, sha) in &want {
        match have.get(path) {
            None => differences.push(format!("missing: {path}")),
            Some(other) if other != sha => differences.push(format!("different: {path}")),
            Some(_) => {}
        }
    }
    for path in have.keys() {
        if !want.contains_key(path) {
            differences.push(format!("extra: {path}"));
        }
    }
    differences.truncate(limit);
    Ok(differences)
}

/// The zip's own files that are Mojang's (the jar's or an object's bytes): none may be.
pub fn foreign(engine: &Path, minecraft: &Path, version: &str) -> Result<Vec<String>, String> {
    let mut mojang = HashSet::new();
    let jar = client_jar(minecraft, version);
    let mut archive =
        zip::ZipArchive::new(File::open(&jar).map_err(|e| format!("{}: {e}", jar.display()))?)
            .map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_dir() {
            mojang.insert(sha256_of(entry).map_err(|e| e.to_string())?);
        }
    }
    if let Ok(prefixes) = fs::read_dir(minecraft.join("assets/objects")) {
        for prefix in prefixes.flatten() {
            for object in fs::read_dir(prefix.path()).into_iter().flatten().flatten() {
                mojang.insert(sha256_file(&object.path())?);
            }
        }
    }
    let mut zip = zip::ZipArchive::new(File::open(engine).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        if name != RECIPE && mojang.contains(&sha256_of(entry).map_err(|e| e.to_string())?) {
            found.push(name);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-ios-prepare-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// A preparation, a client jar and an asset object: what `pack` keeps and what `assemble`
    /// brings back from the official files.
    #[test]
    fn a_preparation_comes_back_byte_for_byte() {
        let dir = scratch("roundtrip");
        let (resources, minecraft) = (dir.join("resources"), dir.join("minecraft"));
        let jar = client_jar(&minecraft, "26.3");
        fs::create_dir_all(jar.parent().unwrap()).unwrap();
        {
            let mut zip = zip::ZipWriter::new(File::create(&jar).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("assets/minecraft/textures/stone.png", options)
                .unwrap();
            zip.write_all(b"stone").unwrap();
            zip.start_file("data/minecraft/tags/block/logs.json", options)
                .unwrap();
            zip.write_all(b"{\"values\":[]}").unwrap();
            zip.finish().unwrap();
        }
        let sound = b"an ogg";
        let sha1 = format!("{:x}", sha1::Sha1::digest(sound));
        write(&object_path(&minecraft, &sha1), sound);
        write(
            &resources.join("assets/minecraft/textures/stone.png"),
            b"stone",
        );
        write(
            &resources.join("data/minecraft/tags/block/logs.json"),
            b"{\"values\":[]}",
        );
        write(&resources.join("assets/minecraft/sounds/step.ogg"), sound);
        write(&resources.join("horizon-resources.json"), b"{\"schema\":7}");
        write(&resources.join("facts.json"), b"[1, 2]");
        write(&resources.join("export.log"), b"work");
        write(&resources.join("classes/Oracle.class"), b"work");
        let engine = dir.join("engine-data.zip");
        let packed = pack(&resources, &minecraft, "26.3", &engine).unwrap();
        assert_eq!(
            packed,
            Packed {
                generated: 2,
                from_jar: 2,
                from_objects: 1
            }
        );
        assert!(foreign(&engine, &minecraft, "26.3").unwrap().is_empty());
        let built = dir.join("built");
        assemble(&engine, &minecraft, &built, &|_, _| {}).unwrap();
        assert!(compare(&resources, &built, 10).unwrap().is_empty());
        assert!(!built.join("export.log").exists());
        // A damaged object is fetched again, never laid out: here, with no such object at
        // Mojang, that is an error rather than a wrong file.
        write(&object_path(&minecraft, &sha1), b"damaged");
        assert!(assemble(&engine, &minecraft, &dir.join("again"), &|_, _| {}).is_err());
        assert!(!dir.join("again").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_files_and_unsafe_paths() {
        assert!(work_file("export.log"));
        assert!(work_file("gameplay-facts/classes/a/B.class"));
        assert!(work_file("horizon-data.bin"));
        assert!(!work_file("facts.json"));
        assert!(!work_file("assets/minecraft/lang/en_us.json"));
        assert!(inside("../escape").is_err());
        assert!(inside("/abs").is_err());
        assert!(inside("assets/ok.png").is_ok());
    }
}
