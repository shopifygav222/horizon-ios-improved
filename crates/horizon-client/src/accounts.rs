//! Starling's Microsoft account inside the game (crate starling-account; a phone has no
//! launcher to sign in for it): the secrets sealed by the platform's keystore (the Android
//! Keystore) in the private folder, the account list beside them, and the session handed to
//! [`crate::auth`] for online-mode servers, renewed in the background before it runs out. The
//! game's official files are fetched from Mojang's servers here too. Without a
//! keystore (a desktop build, `STARLING_ACCOUNTS=1`) the secrets stay in memory for the run.
use starling_account::files::{Pins, Progress, Sources, Wanted};
use starling_account::{Account, Endpoints, MemoryStore, SecretStore, SignIn, SignInState};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

/// How often the background renewal looks at the token (it renews in its last hour).
const RENEW_EVERY: Duration = Duration::from_secs(20 * 60);
/// Written beside the official files once every one of them is fetched and checked.
const FILES_DONE: &str = "starling-files.json";

/// Whether the game signs in itself: on Android, or on a desktop asked to
/// (`STARLING_ACCOUNTS=1`; the launcher signs in there otherwise).
pub fn in_game() -> bool {
    crate::platform::registered() || std::env::var_os("STARLING_ACCOUNTS").is_some_and(|v| v == "1")
}

/// The private folder holding `accounts.json` and the sealed secrets.
pub fn dir() -> PathBuf {
    crate::logging::private_home().join("accounts")
}

/// Where the official files go, in the official launcher's layout (`versions/`, `libraries/`,
/// `assets/`): the private folder, never shared storage.
pub fn files_root() -> PathBuf {
    crate::logging::private_home().join("minecraft")
}

/// The server jar the last complete download fetched, for the preparer (preparer.rs).
pub fn server_jar() -> Option<PathBuf> {
    let done: serde_json::Value =
        serde_json::from_slice(&std::fs::read(files_root().join(FILES_DONE)).ok()?).ok()?;
    done["server"]
        .as_str()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
}

/// Secrets sealed by the platform's keystore, one file each: what lands on the disk opens on
/// this device only.
struct SealedStore {
    dir: PathBuf,
}

impl SealedStore {
    fn path(&self, key: &str) -> Result<PathBuf, String> {
        // Keys are profile UUIDs: hex only, never a path.
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err("not an account key".into());
        }
        Ok(self.dir.join(format!("{key}.sealed")))
    }
}

impl SecretStore for SealedStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let path = self.path(key)?;
        let sealed = match std::fs::read(&path) {
            Ok(sealed) => sealed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        // Sealed by another install's key (the app reinstalled): signed out, not an error.
        Ok(crate::platform::open(&sealed).and_then(|plain| String::from_utf8(plain).ok()))
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let path = self.path(key)?;
        let sealed = crate::platform::seal(secret.as_bytes())
            .ok_or("The device's keystore refused to keep the sign-in.")?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let mut part = path.as_os_str().to_owned();
        part.push(".part");
        std::fs::write(&part, sealed).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &path).map_err(|e| e.to_string())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        match std::fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// The store for this run: sealed where the platform can seal, else memory.
pub fn store() -> Arc<dyn SecretStore> {
    static STORE: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            if crate::platform::can_seal() {
                Arc::new(SealedStore {
                    dir: dir().join("secrets"),
                })
            } else {
                Arc::new(MemoryStore::default())
            }
        })
        .clone()
}

/// Hand `account`'s session to the game: online-mode servers join with it, worlds open with its
/// name.
fn use_session(account: &Account, secrets: &starling_account::Secrets) -> Result<(), String> {
    let uuid = account
        .uuid_u128()
        .ok_or("The Minecraft profile has no valid UUID.")?;
    let mut session = crate::auth::AuthSession::new(&account.name, uuid, secrets.access_token());
    session.xuid = secrets.xuid().map(str::to_owned);
    crate::auth::set_current(session);
    Ok(())
}

/// Where the game's account is: what the account screen shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum State {
    /// Not signed in: the game plays offline (single player, LAN, offline-mode servers).
    #[default]
    Offline,
    /// Renewing the saved sign-in.
    Resuming,
    /// Asking Microsoft for a code.
    Starting,
    /// Waiting for the player at `link` with `code`.
    Code {
        code: String,
        link: String,
    },
    SignedIn(Account),
    /// The last attempt failed: the reason, for the player.
    Failed(String),
}

/// Where the official files are.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Files {
    /// Not fetched (or not all of them).
    #[default]
    Missing,
    Checking,
    Downloading(Progress),
    /// Every file is there and checked.
    Ready,
    Failed(String),
}

#[derive(Default)]
struct Shared {
    state: State,
    files: Files,
    /// Set to stop the sign-in or the download in progress.
    cancel: Option<Arc<AtomicBool>>,
    files_cancel: Option<Arc<AtomicBool>>,
    /// Whether the files' folder was looked at for an earlier download.
    files_seen: bool,
}

fn shared() -> MutexGuard<'static, Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The account now: what the account screen and the title's button show.
pub fn state() -> State {
    shared().state.clone()
}

/// The official files now.
pub fn files() -> Files {
    let mut s = shared();
    if !s.files_seen {
        s.files_seen = true;
        if s.files == Files::Missing && files_root().join(FILES_DONE).exists() {
            s.files = Files::Ready;
        }
    }
    s.files.clone()
}

/// The saved account, renewed in the background (at start), then kept fresh: the game plays as
/// it when ready.
pub fn resume() {
    {
        let mut s = shared();
        if starling_account::Accounts::load(&dir())
            .selected()
            .is_none()
        {
            return;
        }
        s.state = State::Resuming;
    }
    std::thread::spawn(|| {
        let state = match renew() {
            Ok(Some(account)) => State::SignedIn(account),
            Ok(None) => State::Offline,
            Err(e) => {
                log::warn!("Horizon: the saved Microsoft sign-in was not renewed: {e}");
                State::Failed(e)
            }
        };
        set_state_if(&State::Resuming, state);
    });
    keep_renewing();
}

/// The selected account's session, renewed if it runs low, handed to the game.
fn renew() -> Result<Option<Account>, String> {
    match starling_account::session(&Endpoints::live(), store().as_ref(), &dir())? {
        Some((account, secrets)) => use_session(&account, &secrets).map(|()| Some(account)),
        None => Ok(None),
    }
}

/// Replace the state only if it is still `from` (a sign-out meanwhile wins).
fn set_state_if(from: &State, to: State) {
    let mut s = shared();
    if s.state == *from {
        s.state = to;
    }
}

/// One thread for the run that renews the signed-in session before the token runs out (it lasts
/// a day; a phone may stay in a world longer).
fn keep_renewing() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(RENEW_EVERY);
            let signed_in = matches!(state(), State::SignedIn(_));
            if signed_in && let Err(e) = renew() {
                // The token in use stays until it runs out: the next round tries again.
                log::warn!("Horizon: the Microsoft sign-in was not renewed: {e}");
            }
        }
    });
}

/// The platform keeps the game in the foreground while this lives (Android's WorkService): the
/// sign-in waits on the browser, a download on the network, and a phone may cut a background
/// app's internet or end it meanwhile.
struct KeepWorking(&'static str);
impl KeepWorking {
    fn new(reason: &'static str, line: &str) -> Self {
        crate::platform::keep_working(reason, line, true);
        Self(reason)
    }
}
impl Drop for KeepWorking {
    fn drop(&mut self) {
        crate::platform::keep_working(self.0, "", false);
    }
}

/// Start signing in: the device code arrives in [`state`] as [`State::Code`]; when the player
/// is done there, the account is kept and the game plays as it.
pub fn sign_in() {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = shared();
        if let Some(old) = s.cancel.replace(cancel.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        s.state = State::Starting;
    }
    let sign_in = SignIn::start(Endpoints::live(), store(), dir());
    // Asked for now, while the game is in front: Android refuses it from the background.
    let working = KeepWorking::new("sign-in", "Waiting for your Microsoft sign-in");
    std::thread::spawn(move || {
        let _working = working;
        let result = loop {
            if cancel.load(Ordering::Relaxed) {
                sign_in.cancel();
                return;
            }
            match sign_in.state() {
                SignInState::Starting => {}
                SignInState::Code(code) => {
                    let to = State::Code {
                        code: code.user_code,
                        link: code.verification_uri,
                    };
                    let mut s = shared();
                    if !cancel.load(Ordering::Relaxed) && s.state != to {
                        s.state = to;
                    }
                }
                SignInState::Done(result) => break result,
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        // The secrets were just saved: read them back for the session.
        let state = match result.and_then(|_| renew()) {
            Ok(Some(account)) => State::SignedIn(account),
            Ok(None) => State::Failed("The sign-in was not kept.".into()),
            Err(e) => State::Failed(e),
        };
        let mut s = shared();
        if !cancel.load(Ordering::Relaxed) {
            s.state = state;
            s.cancel = None;
        }
    });
    keep_renewing();
}

/// Stop a sign-in in progress.
pub fn cancel() {
    let mut s = shared();
    if let Some(old) = s.cancel.take() {
        old.store(true, Ordering::Relaxed);
    }
    if matches!(s.state, State::Starting | State::Code { .. }) {
        s.state = State::Offline;
    }
}

/// Sign out: the secrets leave the device and the game plays offline.
pub fn sign_out() -> Result<(), String> {
    cancel();
    starling_account::sign_out(store().as_ref(), &dir())?;
    crate::auth::clear_current();
    shared().state = State::Offline;
    Ok(())
}

/// Fetch the official files of the version Starling plays from Mojang's servers, as the
/// official launcher does: after a sign-in, or for a player who chose to play offline, each
/// file checked against its hash, into [`files_root`]. A second call resumes what an
/// interrupted one left.
pub fn download_files() {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = shared();
        if matches!(s.files, Files::Checking | Files::Downloading(_)) {
            return;
        }
        s.files = Files::Checking;
        s.files_cancel = Some(cancel.clone());
    }
    let working = KeepWorking::new("files", "Downloading the game files");
    std::thread::spawn(move || {
        let _working = working;
        let result = (|| {
            let root = files_root();
            let _ = std::fs::remove_file(root.join(FILES_DONE));
            let progress = |p: Progress| {
                let mut s = shared();
                if matches!(s.files, Files::Checking | Files::Downloading(_)) {
                    s.files = Files::Downloading(p);
                }
            };
            // The server jar and the libraries are Java's, which iOS doesn't run (preparer.rs):
            // the app carries what they made.
            let wanted = Wanted {
                server: !cfg!(target_os = "ios"),
                libraries: !cfg!(target_os = "ios"),
                assets: true,
            };
            let pins = Pins::starling();
            let installed = starling_account::files::download(
                &Sources::live(),
                &pins,
                &root,
                wanted,
                &progress,
                &|| cancel.load(Ordering::Relaxed),
            )?;
            let done = serde_json::json!({
                "version": pins.version,
                "client": installed.client_jar,
                "server": installed.server_jar,
            });
            std::fs::write(root.join(FILES_DONE), done.to_string()).map_err(|e| e.to_string())
        })();
        let mut s = shared();
        s.files_cancel = None;
        s.files = match result {
            Ok(()) => Files::Ready,
            Err(_) if cancel.load(Ordering::Relaxed) => Files::Missing,
            Err(e) => {
                log::warn!("Horizon: the game files were not fetched: {e}");
                Files::Failed(e)
            }
        };
    });
}

/// Stop the download in progress (what it fetched stays for the next one).
pub fn cancel_download() {
    if let Some(cancel) = shared().files_cancel.take() {
        cancel.store(true, Ordering::Relaxed);
    }
}
