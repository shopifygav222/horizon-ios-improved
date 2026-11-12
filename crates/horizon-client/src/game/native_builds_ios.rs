//! iOS: native mods (docs/ios.md). iOS runs no code its app did not ship with, so where Starling's
//! Android app swaps in a library its build service compiled (native_builds.rs), a native mod here
//! is compiled into the app itself: this repository's `ios` workflow, run with the mods, builds an
//! .ipa with them (the run's artifact, kept 30 days and never published as a release), which the
//! player installs over this app.
//!
//! The Mods page's top (`ModList::native_game`, drawn as Android's) says which mods this app was
//! built with (`native-mods.json` beside its program), and when the native mods turned on are not
//! all in it, builds the app with them: the player's GitHub token (Actions: read and write on this
//! repository, kept in the Keychain) starts the workflow, the build is followed while they play,
//! and the finished .ipa is downloaded with the same token and handed to the share sheet
//! (SideStore, AltStore, LiveContainer or any other signer), or saved to Files.
use super::Game;
use horizon_client::mod_list::{NOT_BUILT, NativeGame, NativeGameAct, NativePage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where the app was built from: the repository and branch whose `ios` workflow builds it again.
const REPO: &str = match option_env!("HORIZON_IOS_REPO") {
    Some(repo) => repo,
    None => "shopifygav222/horizon-ios-improved",
};
const BRANCH: &str = match option_env!("HORIZON_IOS_BRANCH") {
    Some(branch) => branch,
    None => "starling-ios",
};
const WORKFLOW: &str = "ios.yml";
const API: &str = "https://api.github.com";
/// The token, in the Keychain: this app's, on this device.
const KEYCHAIN_SERVICE: &str = "dev.horizon.starling.github";
const KEYCHAIN_ACCOUNT: &str = "actions-token";
/// The build being followed, in the private folder: it survives a restart of the app.
const PENDING_FILE: &str = "native-build.json";
/// What the program was built with, beside it (game/mods.rs BUILD_INFO).
const BUILD_INFO: &str = "native-mods.json";
/// Where the finished .ipa is kept until it is installed: the private folder's.
const BUILDS_DIR: &str = "native-builds";
/// The largest download taken for an .ipa's artifact.
const LARGEST_ARTIFACT: u64 = 512 << 20;
/// How often a build under way is looked at.
const POLL: Duration = Duration::from_secs(20);
/// A build that has not shown up as a run by then was not started.
const FIND_RUN: Duration = Duration::from_secs(5 * 60);

/// A build the player asked for, as kept in [`PENDING_FILE`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Pending {
    /// The mod ids, sorted: the workflow's `mods` input.
    mods: Vec<String>,
    /// When it was asked for (seconds since 1970): its run is the first one after.
    asked: u64,
    run: Option<u64>,
    number: Option<u64>,
    page: Option<String>,
    /// The finished .ipa, downloaded into [`BUILDS_DIR`].
    ipa: Option<String>,
    failed: Option<String>,
}

#[derive(Default)]
struct Shared {
    pending: Option<Pending>,
    /// How the build is going, in words.
    progress: String,
    /// A message for the player (and whether it is a warning), until the next action.
    message: Option<(String, bool)>,
    /// A thread follows the build.
    following: bool,
    /// Bumped on every change, so the page is redrawn once per change.
    changes: u64,
}

#[derive(Default)]
pub(super) struct NativeBuilds {
    shared: Arc<Mutex<Shared>>,
    shown: u64,
    loaded: bool,
}

fn shared(builds: &NativeBuilds) -> std::sync::MutexGuard<'_, Shared> {
    builds.shared.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn pending_path() -> std::path::PathBuf {
    horizon_client::logging::private_home().join(PENDING_FILE)
}

fn save_pending(pending: Option<&Pending>) {
    let path = pending_path();
    match pending {
        Some(p) => {
            if let Ok(text) = serde_json::to_string_pretty(p) {
                let partial = path.with_extension("partial");
                if std::fs::write(&partial, text).is_ok() {
                    let _ = std::fs::rename(partial, path);
                }
            }
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn load_pending() -> Option<Pending> {
    let text = std::fs::read_to_string(pending_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn builds_dir() -> std::path::PathBuf {
    horizon_client::logging::private_home().join(BUILDS_DIR)
}

/// The downloaded builds go: once installed or dismissed, nothing of them stays.
fn forget_builds() {
    let _ = std::fs::remove_dir_all(builds_dir());
}

fn token() -> Option<String> {
    let bytes = horizon_gamecontroller::keychain_get(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .ok()
        .flatten()?;
    String::from_utf8(bytes).ok().filter(|t| !t.is_empty())
}

/// A GitHub token, as GitHub makes them: a prefix and letters, digits and underscores.
fn looks_like_token(text: &str) -> bool {
    (text.starts_with("github_pat_") || text.starts_with("ghp_"))
        && (20..=255).contains(&text.len())
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The native mods this app was built with: (id, version), from [`BUILD_INFO`] beside its program.
pub(super) fn built_mods() -> Vec<(String, String)> {
    let info: Value = std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::read_to_string(exe.with_file_name(BUILD_INFO)).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    info["mods"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some((
                m["id"].as_str()?.to_owned(),
                m["version"].as_str().unwrap_or("?").to_owned(),
            ))
        })
        .collect()
}

/// [`built_mods`] as `native-mods.json` holds it, for the Mods page's rows (game/mods.rs).
pub(super) fn running_build_info() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    std::fs::read_to_string(exe.with_file_name(BUILD_INFO)).ok()
}

/// The ids of the native mods in the `horizon-native-mods` folder that are turned on.
fn folder_mods_on(off: &BTreeSet<String>) -> BTreeSet<String> {
    let folder = Game::game_dir().join(horizon_client::mod_list::NATIVE_DIR);
    std::fs::read_dir(folder)
        .map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "hnmod"))
        .filter_map(|p| horizon_mods::index::native::Bundle::read(&p).ok())
        .map(|b| b.manifest.id)
        .filter(|id| !off.contains(id))
        .collect()
}

fn mods_label(mods: &[String]) -> String {
    match mods {
        [] => "no native mods".to_owned(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A form's value: every byte but the unreserved ones escaped.
fn escape(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Seconds since 1970 of GitHub's time (`2026-11-12T16:58:38Z`).
fn unix_time(text: &str) -> Option<u64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hh, mm, ss) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// What GitHub answered, read as JSON, or why not, in the player's words.
fn github(method: &str, path: &str, token: &str, body: Option<&str>) -> Result<Value, String> {
    let request = ureq::request(method, &format!("{API}{path}"))
        .timeout(Duration::from_secs(30))
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .set("User-Agent", "starling-ios")
        .set("Authorization", &format!("Bearer {token}"));
    let response = match body {
        Some(body) => request
            .set("Content-Type", "application/json")
            .send_string(body),
        None => request.call(),
    };
    match response {
        Ok(response) => {
            let text = response.into_string().map_err(|e| e.to_string())?;
            Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
        }
        Err(ureq::Error::Status(code, response)) => {
            let said = response
                .into_string()
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_default();
            Err(match code {
                401 => {
                    "GitHub doesn't accept the token (expired or copied wrong): paste a new one."
                        .to_owned()
                }
                403 if said.contains("rate limit") => {
                    "GitHub asks to wait a little (too many requests): try again in a few minutes."
                        .to_owned()
                }
                403 | 404 => format!(
                    "The token may not run {REPO}'s Actions: make it for that repository, with \
                     Actions: Read and write ({code})."
                ),
                _ => format!("GitHub said {code}: {said}"),
            })
        }
        Err(e) => Err(format!("GitHub can't be reached: {e}")),
    }
}

/// Starts the workflow with `mods`, then follows its run to its .ipa, downloaded, keeping
/// [`Shared`] (and [`PENDING_FILE`]) up to date. Runs on its own thread.
fn follow(shared: Arc<Mutex<Shared>>, start: bool) {
    let set = |f: &dyn Fn(&mut Shared)| {
        let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut s);
        s.changes += 1;
        save_pending(s.pending.as_ref());
    };
    let pending = || {
        shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .clone()
    };
    let fail = |why: String| {
        set(&|s| {
            if let Some(p) = &mut s.pending {
                p.failed = Some(why.clone());
            }
            s.following = false;
        })
    };
    let Some(token) = token() else {
        return fail("There is no GitHub token on this device: paste one first.".to_owned());
    };
    if start {
        let Some(p) = pending() else { return };
        set(&|s| s.progress = "Asking GitHub to build it...".to_owned());
        let body = serde_json::json!({"ref": BRANCH, "inputs": {"mods": p.mods.join(" ")}});
        let path = format!("/repos/{REPO}/actions/workflows/{WORKFLOW}/dispatches");
        if let Err(why) = github("POST", &path, &token, Some(&body.to_string())) {
            return fail(why);
        }
    }
    loop {
        let Some(p) = pending() else { return };
        if p.failed.is_some() || p.ipa.is_some() {
            set(&|s| s.following = false);
            return;
        }
        let step = match p.run {
            None => find_run(&p, &token),
            Some(run) => check_run(run, &token),
        };
        match step {
            Ok(Step::Wait(progress)) => set(&|s| s.progress = progress.clone()),
            Ok(Step::Found { run, number, page }) => set(&|s| {
                if let Some(p) = &mut s.pending {
                    p.run = Some(run);
                    p.number = Some(number);
                    p.page = Some(page.clone());
                }
                s.progress = "GitHub is about to build it.".to_owned();
            }),
            Ok(Step::Artifact { id, size }) => {
                let number = p.number.unwrap_or_default();
                let said = |progress: String| set(&|s| s.progress = progress.clone());
                match download(id, size, number, &token, &said) {
                    Ok(ipa) => {
                        return set(&|s| {
                            if let Some(p) = &mut s.pending {
                                p.ipa = Some(ipa.to_string_lossy().into_owned());
                            }
                            s.following = false;
                        });
                    }
                    Err(why) => set(&|s| {
                        s.progress = format!("The download stopped ({why}): trying again.");
                    }),
                }
            }
            Err(Fatal(why)) => return fail(why),
            Ok(Step::Retry(why)) => {
                set(&|s| s.progress = format!("Waiting for GitHub ({why})."));
            }
        }
        std::thread::sleep(POLL);
    }
}

enum Step {
    Wait(String),
    Found {
        run: u64,
        number: u64,
        page: String,
    },
    /// The run built: its artifact, the .ipa in a zip.
    Artifact { id: u64, size: u64 },
    /// GitHub can't be reached now: asked again at the next look.
    Retry(String),
}
struct Fatal(String);

/// The run the workflow started for `p`: the first after it was asked for, named with its mods.
fn find_run(p: &Pending, token: &str) -> Result<Step, Fatal> {
    let path = format!(
        "/repos/{REPO}/actions/workflows/{WORKFLOW}/runs?event=workflow_dispatch&branch={}&per_page=20",
        escape(BRANCH)
    );
    let runs = match github("GET", &path, token, None) {
        Ok(runs) => runs,
        Err(why) => return Ok(Step::Retry(why)),
    };
    let name = format!("native mods {} ·", p.mods.join(" "));
    let found = runs["workflow_runs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|run| {
            run["display_title"]
                .as_str()
                .is_some_and(|t| t.contains(&name))
                && run["created_at"]
                    .as_str()
                    .and_then(unix_time)
                    .is_some_and(|at| at + 120 >= p.asked)
        });
    match found {
        Some(run) => Ok(Step::Found {
            run: run["id"].as_u64().unwrap_or_default(),
            number: run["run_number"].as_u64().unwrap_or_default(),
            page: run["html_url"].as_str().unwrap_or_default().to_owned(),
        }),
        None if now().saturating_sub(p.asked) > FIND_RUN.as_secs() => Err(Fatal(
            "GitHub accepted the build but never started it. See the repository's Actions page."
                .to_owned(),
        )),
        None => Ok(Step::Wait("GitHub is starting the build...".to_owned())),
    }
}

fn check_run(run: u64, token: &str) -> Result<Step, Fatal> {
    let state = match github(
        "GET",
        &format!("/repos/{REPO}/actions/runs/{run}"),
        token,
        None,
    ) {
        Ok(state) => state,
        Err(why) => return Ok(Step::Retry(why)),
    };
    let started = state["run_started_at"]
        .as_str()
        .or(state["created_at"].as_str())
        .and_then(unix_time)
        .unwrap_or_else(now);
    let minutes = now().saturating_sub(started) / 60;
    match state["status"].as_str().unwrap_or_default() {
        "completed" => {}
        "queued" | "waiting" | "requested" | "pending" => {
            return Ok(Step::Wait(format!(
                "Waiting for one of GitHub's Macs ({minutes} min so far)."
            )));
        }
        _ => {
            return Ok(Step::Wait(format!(
                "Building on GitHub: {minutes} min so far, usually 30 to 60. Keep playing."
            )));
        }
    }
    match state["conclusion"].as_str().unwrap_or_default() {
        "success" => {}
        "cancelled" => return Err(Fatal("The build was cancelled on GitHub.".to_owned())),
        _ => {
            return Err(Fatal(
                "The app didn't build with these mods: one may not fit this Starling. See the build \
                 for which, turn it off, and build again."
                    .to_owned(),
            ));
        }
    }
    let path = format!("/repos/{REPO}/actions/runs/{run}/artifacts");
    let artifacts = match github("GET", &path, token, None) {
        Ok(artifacts) => artifacts,
        Err(why) => return Ok(Step::Retry(why)),
    };
    let found = artifacts["artifacts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| n.ends_with("-native-mods"))
        });
    match found {
        Some(a) if a["expired"].as_bool() == Some(true) => Err(Fatal(
            "GitHub no longer keeps that build (30 days): build again.".to_owned(),
        )),
        Some(a) => Ok(Step::Artifact {
            id: a["id"].as_u64().unwrap_or_default(),
            size: a["size_in_bytes"].as_u64().unwrap_or_default(),
        }),
        None => Err(Fatal(
            "The build finished without an .ipa to install.".to_owned(),
        )),
    }
}

/// The run's artifact (`size` bytes, a zip holding the .ipa) downloaded with the token, and its
/// .ipa taken out into [`builds_dir`] as `Starling-native-r<number>.ipa`. GitHub answers with
/// a link to its storage, which ureq follows without the token.
fn download(
    id: u64,
    size: u64,
    number: u64,
    token: &str,
    said: &dyn Fn(String),
) -> Result<std::path::PathBuf, String> {
    use std::io::{Read, Write};
    if size > LARGEST_ARTIFACT {
        return Err(format!("{} MB is more than a build", size >> 20));
    }
    let dir = builds_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(60))
        .build();
    let response = agent
        .get(&format!("{API}/repos/{REPO}/actions/artifacts/{id}/zip"))
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .set("User-Agent", "starling-ios")
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| e.to_string())?;
    let zip_path = dir.join("build.zip");
    let mut file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut reader = response.into_reader().take(LARGEST_ARTIFACT + 1);
    let mut buffer = vec![0; 1 << 16];
    let (mut done, mut shown) = (0u64, 0u64);
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if done >> 20 != shown {
            shown = done >> 20;
            said(format!(
                "Built. Downloading it: {shown} of {} MB.",
                size.div_ceil(1 << 20)
            ));
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let ipa = dir.join(format!("Starling-native-r{number}.ipa"));
    let archive = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(archive).map_err(|e| e.to_string())?;
    let name = archive
        .file_names()
        .find(|n| n.ends_with(".ipa") && !n.contains('/'))
        .map(str::to_owned)
        .ok_or("the build holds no .ipa")?;
    let mut entry = archive.by_name(&name).map_err(|e| e.to_string())?;
    let mut out = std::fs::File::create(&ipa).map_err(|e| e.to_string())?;
    std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    out.sync_all().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip_path);
    Ok(ipa)
}

impl Game<'_> {
    /// The native mods turned on in the folder that this app doesn't have yet are chosen for the
    /// next build (a box on their rows), and the page's top says what the app has and offers it.
    pub(super) fn offer_compile(&mut self) {
        for row in &mut self.settings.mods.installed {
            if row.problem.as_deref() == Some(NOT_BUILT) {
                row.native = Some(NativePage::default());
            }
        }
        self.settings.mods.native_game = self.native_game();
    }

    /// The mods a build is made with: those turned on in the folder, and the ones this app has
    /// that are still on (a mod built in from elsewhere stays in).
    fn wanted_mods(&self) -> (BTreeSet<String>, Vec<String>) {
        let off = &self.settings.mods.disabled;
        let wanted = folder_mods_on(off);
        let mut build: BTreeSet<String> = wanted.clone();
        build.extend(
            built_mods()
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| !off.contains(id)),
        );
        (wanted, build.into_iter().collect())
    }

    /// The Mods page's top: what this app was built with, then the build of the mods turned on
    /// that it lacks, from the token to the install.
    fn native_game(&self) -> Option<NativeGame> {
        let built = built_mods();
        let (wanted, build) = self.wanted_mods();
        let state = shared(&self.native_builds);
        let pending = state.pending.clone();
        if built.is_empty() && wanted.is_empty() && pending.is_none() {
            return None;
        }
        let mut game = NativeGame::default();
        let version = crate::android::app_version().map_or_else(String::new, |(v, _)| v);
        let ids: Vec<String> = built.iter().map(|(id, v)| format!("{id} {v}")).collect();
        game.lines.push((
            if built.is_empty() {
                format!("This app: Starling {version} without native mods.")
            } else {
                format!("This app: Starling {version} with {}.", mods_label(&ids))
            },
            false,
        ));
        let have: BTreeSet<String> = built.into_iter().map(|(id, _)| id).collect();
        let missing = !wanted.is_subset(&have);
        if let Some(p) = &pending {
            let mods = mods_label(&p.mods);
            if let Some(why) = &p.failed {
                game.lines
                    .push((format!("The build with {mods} failed. {why}"), true));
                if p.page.is_some() {
                    game.buttons
                        .push(("See the build".to_owned(), NativeGameAct::ShowCompile));
                }
                game.buttons
                    .push(("Dismiss".to_owned(), NativeGameAct::PlayWithout));
            } else if p.ipa.is_some() {
                game.lines.push((
                    format!(
                        "Built with {mods} and downloaded. Install it over this app: your worlds, \
                         settings and accounts stay. Install opens the share sheet: pick \
                         SideStore, AltStore, LiveContainer or the app you sideload with."
                    ),
                    false,
                ));
                game.buttons
                    .push(("Install...".to_owned(), NativeGameAct::Install));
                game.buttons
                    .push(("Save to Files".to_owned(), NativeGameAct::SaveIpa));
                game.buttons
                    .push(("Dismiss".to_owned(), NativeGameAct::PlayWithout));
            } else {
                game.lines
                    .push((format!("Building the app with {mods}."), false));
                if !state.progress.is_empty() {
                    game.lines.push((state.progress.clone(), false));
                }
                if p.page.is_some() {
                    game.buttons
                        .push(("See the build".to_owned(), NativeGameAct::ShowCompile));
                }
                game.buttons
                    .push(("Stop following".to_owned(), NativeGameAct::PlayWithout));
            }
        }
        let building = pending
            .as_ref()
            .is_some_and(|p| p.failed.is_none() && p.ipa.is_none());
        if missing && !building {
            game.lines.push((
                format!(
                    "The native mods turned on aren't all in this app. On iOS they are compiled \
                     into it: GitHub builds it with {} (about 30 to 60 minutes).",
                    mods_label(&build)
                ),
                false,
            ));
            if token().is_some() {
                game.buttons
                    .push(("Build on GitHub".to_owned(), NativeGameAct::Compile));
                game.buttons
                    .push(("New token".to_owned(), NativeGameAct::Token));
            } else {
                game.lines.push((
                    format!(
                        "First, a GitHub token: on github.com, Settings > Developer settings > \
                         Fine-grained tokens > Generate, for {REPO} only, with Actions: Read and \
                         write. Copy it, then paste it here (it stays in this device's Keychain)."
                    ),
                    false,
                ));
                game.buttons
                    .push(("Paste GitHub token".to_owned(), NativeGameAct::Token));
            }
        }
        if let Some((text, warning)) = &state.message {
            game.lines.push((text.clone(), *warning));
        }
        Some(game)
    }

    fn refresh_native_game(&mut self) {
        self.settings.mods.native_game = self.native_game();
    }

    /// A native mod was turned on or off: the build it needs may be another.
    pub(super) fn native_mod_toggled(&mut self, i: usize) {
        if self
            .settings
            .mods
            .installed
            .get(i)
            .is_some_and(|m| m.native.is_some())
        {
            self.refresh_native_game();
        }
    }

    /// A button of the Mods page's top.
    pub(super) fn native_game_act(&mut self, act: NativeGameAct) {
        let ipa = shared(&self.native_builds)
            .pending
            .as_ref()
            .and_then(|p| p.ipa.clone())
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_file());
        let mut message = None;
        match act {
            NativeGameAct::Compile => self.start_build(),
            NativeGameAct::ShowCompile => {
                let page = shared(&self.native_builds)
                    .pending
                    .as_ref()
                    .and_then(|p| p.page.clone())
                    .unwrap_or_else(|| format!("https://github.com/{REPO}/actions"));
                horizon_gamecontroller::open_url(&page);
            }
            NativeGameAct::PlayWithout => {
                let mut state = shared(&self.native_builds);
                state.pending = None;
                state.progress.clear();
                state.changes += 1;
                save_pending(None);
                forget_builds();
            }
            NativeGameAct::Token => {
                let pasted = horizon_gamecontroller::clipboard_text().unwrap_or_default();
                let pasted = pasted.trim();
                message = Some(if !looks_like_token(pasted) {
                    (
                        "The clipboard holds no GitHub token: copy the token GitHub showed \
                         (github_pat_...), then paste it again."
                            .to_owned(),
                        true,
                    )
                } else if horizon_gamecontroller::keychain_set(
                    KEYCHAIN_SERVICE,
                    KEYCHAIN_ACCOUNT,
                    pasted.as_bytes(),
                )
                .is_ok()
                {
                    ("Token saved in the Keychain.".to_owned(), false)
                } else {
                    (
                        "The Keychain didn't take the token: try again.".to_owned(),
                        true,
                    )
                });
            }
            NativeGameAct::Install | NativeGameAct::SaveIpa => {
                let opened = ipa.as_deref().is_some_and(|path| {
                    if act == NativeGameAct::Install {
                        horizon_gamecontroller::share(path)
                    } else {
                        horizon_gamecontroller::export(&[path.to_owned()])
                    }
                });
                if !opened {
                    message = Some((
                        "The .ipa couldn't be opened here: press Dismiss and build again."
                            .to_owned(),
                        true,
                    ));
                }
            }
            _ => {}
        }
        {
            let mut state = shared(&self.native_builds);
            state.message = message;
            state.changes += 1;
        }
        self.refresh_native_game();
    }

    /// Asks GitHub to build the app with the mods turned on, and follows the build.
    fn start_build(&mut self) {
        let (_, build) = self.wanted_mods();
        if build.is_empty() {
            return;
        }
        let shared_state = self.native_builds.shared.clone();
        {
            let mut state = shared(&self.native_builds);
            if state.following {
                return;
            }
            state.pending = Some(Pending {
                mods: build,
                asked: now(),
                ..Pending::default()
            });
            state.progress = "Asking GitHub to build it...".to_owned();
            state.message = None;
            state.following = true;
            state.changes += 1;
            save_pending(state.pending.as_ref());
        }
        let _ = std::thread::Builder::new()
            .name("native build".into())
            .spawn(move || follow(shared_state, true));
    }

    /// A row's own button: a native mod not in this app builds it, as the page's top does.
    pub(super) fn native_builds_button(&mut self, i: usize) -> bool {
        let not_built = self
            .settings
            .mods
            .installed
            .get(i)
            .is_some_and(|row| row.problem.as_deref() == Some(NOT_BUILT));
        if not_built {
            self.native_game_act(NativeGameAct::Compile);
        }
        not_built
    }

    /// Every frame: the build asked for before the app last closed is followed again, and the
    /// page's top is redrawn when the build moved on.
    pub(super) fn poll_native_builds(&mut self) {
        if !self.native_builds.loaded {
            self.native_builds.loaded = true;
            let mut pending = load_pending();
            // The app now running is that build's: done.
            let have: Vec<String> = built_mods().into_iter().map(|(id, _)| id).collect();
            if pending
                .as_ref()
                .is_some_and(|p| p.ipa.is_some() && p.mods == have)
            {
                pending = None;
                save_pending(None);
                forget_builds();
            }
            // A downloaded .ipa that went (iOS clears caches; it is not one) comes again.
            if let Some(p) = &mut pending
                && p.ipa.as_ref().is_some_and(|f| !std::path::Path::new(f).is_file())
            {
                p.ipa = None;
            }
            let follow_it = pending
                .as_ref()
                .is_some_and(|p| p.failed.is_none() && p.ipa.is_none());
            {
                let mut state = shared(&self.native_builds);
                state.pending = pending;
                state.following = follow_it;
                state.changes += 1;
            }
            if follow_it {
                let shared_state = self.native_builds.shared.clone();
                let _ = std::thread::Builder::new()
                    .name("native build".into())
                    .spawn(move || follow(shared_state, false));
            }
        }
        let changes = shared(&self.native_builds).changes;
        if changes != self.native_builds.shown {
            self.native_builds.shown = changes;
            if self.settings.mods.native_game.is_some() || self.native_game().is_some() {
                self.refresh_native_game();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_times_read_as_seconds() {
        assert_eq!(unix_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_time("2026-11-12T16:58:38Z"), Some(1_794_502_718));
        assert_eq!(unix_time("2000-02-29T12:00:00Z"), Some(951_825_600));
        assert_eq!(unix_time("nonsense"), None);
    }

    #[test]
    fn tokens_are_told_from_other_text() {
        assert!(looks_like_token("github_pat_11ABCDEFG0123456789_abcdefXYZ"));
        assert!(looks_like_token("ghp_0123456789abcdefABCDEF0123456789abcd"));
        assert!(!looks_like_token("github_pat_ has a space"));
        assert!(!looks_like_token("hello"));
    }

    #[test]
    fn query_values_are_escaped() {
        assert_eq!(escape("starling-ios"), "starling-ios");
        assert_eq!(escape("feature/ios mods"), "feature%2Fios%20mods");
    }

    #[test]
    fn mod_lists_read_as_words() {
        assert_eq!(mods_label(&[]), "no native mods");
        assert_eq!(mods_label(&["a".into()]), "a");
        assert_eq!(
            mods_label(&["a".into(), "b".into(), "c".into()]),
            "a, b and c"
        );
    }
}
