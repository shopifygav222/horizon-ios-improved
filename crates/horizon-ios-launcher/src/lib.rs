//! The phone's launcher, in the game's own bundle. The game owns the app (winit runs UIKit's main loop and
//! never hands it back), so the launcher cannot run before it: Xcode embeds the SwiftUI launcher as
//! `Frameworks/HorizonLauncherKit.framework`, and once the game's window is up [`present`] loads it and asks
//! it to cover the game. Its Play button uncovers the game, at its title screen.
//!
//! Loaded at run time rather than linked, so a bundle without the framework (the workflow's hand-assembled
//! one, a desktop build) is the game as before. The game forbids `unsafe`, so the loading lives here.
//!
//! The launcher talks to the game in the game's own control language (docs/horizon-ctl-commands.md): it
//! hands [`present`]'s caller a line such as `worlds` or `open My World`, and the game answers with the JSON
//! the control socket would. The two share one thread, the main one, so nothing here waits: a line is queued
//! ([`take`]), the game answers it on its next frame, and the answer goes back to Swift through the callback
//! that came with it.

use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
};

/// One line from the launcher, and the way back to it.
pub struct Request {
    pub line: String,
    reply: Option<Box<dyn FnOnce(&str) + Send>>,
}

impl Request {
    pub fn new(line: impl Into<String>, reply: impl FnOnce(&str) + Send + 'static) -> Self {
        Self {
            line: line.into(),
            reply: Some(Box::new(reply)),
        }
    }

    pub fn answer(mut self, json: &str) {
        if let Some(reply) = self.reply.take() {
            reply(json);
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            reply(r#"{"error":"the game dropped this request"}"#);
        }
    }
}

static QUEUE: Mutex<VecDeque<Request>> = Mutex::new(VecDeque::new());

#[cfg(target_os = "ios")]
const ENABLED: bool = true;

static COVERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_covered(covered: bool) {
    COVERED.store(covered, std::sync::atomic::Ordering::Relaxed);
}

pub fn covered() -> bool {
    COVERED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn push(request: Request) {
    QUEUE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(request);
}

pub fn take() -> Vec<Request> {
    QUEUE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .collect()
}

static STARTED: (Mutex<Option<bool>>, std::sync::Condvar) =
    (Mutex::new(None), std::sync::Condvar::new());

pub fn wait_for_start() -> bool {
    let (started, signal) = &STARTED;
    let mut started = started.lock().unwrap_or_else(PoisonError::into_inner);
    let open = *started.get_or_insert_with(|| !bundled());
    if open {
        return false;
    }
    while *started != Some(true) {
        started = signal.wait(started).unwrap_or_else(PoisonError::into_inner);
    }
    true
}

pub fn start() {
    let (started, signal) = &STARTED;
    *started.lock().unwrap_or_else(PoisonError::into_inner) = Some(true);
    signal.notify_all();
}

pub fn reset() {
    *STARTED.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(!bundled());
}

pub fn waiting() -> bool {
    let started = STARTED.0.lock().unwrap_or_else(PoisonError::into_inner);
    started.is_none_or(|open| !open) && bundled()
}

fn bundled() -> bool {
    #[cfg(target_os = "ios")]
    return ENABLED && ios::framework().is_some();
    #[cfg(not(target_os = "ios"))]
    false
}

pub fn present() -> bool {
    static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SHOWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let shown = show();
    if !shown {
        start();
    }
    shown
}

pub fn show() -> bool {
    #[cfg(target_os = "ios")]
    return ENABLED && ios::present();
    #[cfg(not(target_os = "ios"))]
    false
}

pub fn present_download() -> bool {
    #[cfg(target_os = "ios")]
    return ios::call(ios::DOWNLOAD);
    #[cfg(not(target_os = "ios"))]
    false
}

#[cfg(target_os = "ios")]
mod ios {
    use std::ffi::{CStr, CString, c_char, c_void};

    const SYMBOL: &CStr = c"horizon_launcher_present";
    pub const DOWNLOAD: &CStr = c"horizon_download_present";

    type Done = extern "C" fn(ctx: *mut c_void, reply: *const c_char);
    type Submit = extern "C" fn(line: *const c_char, ctx: *mut c_void, done: Done);

    extern "C" fn submit(line: *const c_char, ctx: *mut c_void, done: Done) {
        let line = if line.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(line) }
                .to_string_lossy()
                .into_owned()
        };
        let ctx = ctx as usize;
        super::push(super::Request::new(line, move |json: &str| {
            let text = CString::new(json.replace('\0', "")).unwrap_or_default();
            done(ctx as *mut c_void, text.as_ptr());
        }));
    }

    pub fn framework() -> Option<CString> {
        let framework = std::env::current_exe()
            .ok()?
            .parent()?
            .join("Frameworks/HorizonLauncherKit.framework/HorizonLauncherKit");
        framework
            .exists()
            .then(|| CString::new(framework.into_os_string().into_encoded_bytes()).ok())?
    }

    pub fn present() -> bool {
        call(SYMBOL)
    }

    pub fn call(symbol_name: &CStr) -> bool {
        let Some(path) = framework() else {
            log::info!("Horizon: no launcher framework in this bundle");
            return false;
        };
        unsafe {
            let handle = libc::dlopen(path.as_ptr(), libc::RTLD_NOW);
            if handle.is_null() {
                log::warn!("Horizon: the launcher framework did not load");
                return false;
            }
            let symbol = libc::dlsym(handle, symbol_name.as_ptr());
            if symbol.is_null() {
                log::warn!("Horizon: the launcher framework has no {symbol_name:?}");
                return false;
            }
            let present: extern "C" fn(Submit) -> bool = std::mem::transmute(symbol);
            present(submit)
        }
    }
}
