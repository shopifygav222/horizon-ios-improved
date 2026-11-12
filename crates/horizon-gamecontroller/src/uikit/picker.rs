//! UIDocumentPickerViewController, Files' own sheet: files picked to import (iOS copies them into
//! the app first), a folder opened where it is (lent with security-scoped access while it is
//! read), and files exported to a folder the player chooses.
use super::key_window;
use crate::{Pick, Picked};
use objc2::{
    MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyObject, NSObject},
};
use objc2_foundation::NSString;
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;

/// `UIDocumentPickerMode`'s values.
const IMPORT: isize = 0;
const OPEN: isize = 1;
const EXPORT_TO_SERVICE: isize = 2;

thread_local! {
    /// The open picker's delegate (UIKit holds delegates weakly) and what its pick goes to.
    static PENDING: RefCell<Option<(Retained<Delegate>, Option<Picked>)>> =
        const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` implements no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HorizonDocumentPickerDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn picked(&self, _controller: &AnyObject, urls: &AnyObject) {
            finish(Some(urls));
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _controller: &AnyObject) {
            finish(None);
        }
    }
);

/// NSURLs handed to the thread that reads them: NSURL is immutable and safe to use from any
/// thread. Dropped, they give back the access iOS lent.
struct Lent(Vec<Retained<AnyObject>>);

// SAFETY: NSURL objects are immutable; retain, release and the security-scoped access calls are
// thread-safe.
unsafe impl Send for Lent {}

impl Drop for Lent {
    fn drop(&mut self) {
        for url in &self.0 {
            // SAFETY: a live NSURL whose access `finish` started.
            let _: () = unsafe { msg_send![&**url, stopAccessingSecurityScopedResource] };
        }
    }
}

fn finish(urls: Option<&AnyObject>) {
    let Some((_delegate, picked)) = PENDING.with(|p| p.borrow_mut().take()) else {
        return;
    };
    let mut lent = Vec::new();
    let mut paths = Vec::new();
    if let Some(urls) = urls {
        // SAFETY: an NSArray of NSURLs UIKit passed for the length of the call; each URL kept is
        // retained.
        unsafe {
            let count: usize = msg_send![urls, count];
            for i in 0..count {
                let url: Option<Retained<AnyObject>> = msg_send![urls, objectAtIndex: i];
                let Some(url) = url else { continue };
                let _: bool = msg_send![&*url, startAccessingSecurityScopedResource];
                let path: Option<Retained<NSString>> = msg_send![&*url, path];
                if let Some(path) = path {
                    paths.push(PathBuf::from(path.to_string()));
                }
                lent.push(url);
            }
        }
    }
    let lent = Lent(lent);
    let Some(picked) = picked else { return };
    let _ = std::thread::Builder::new()
        .name("horizon-picked".into())
        .spawn(move || {
            picked(paths);
            drop(lent);
        });
}

/// The view controller showing now: the root one, or what it presents over itself.
fn topmost(mut controller: Retained<AnyObject>) -> Retained<AnyObject> {
    loop {
        // SAFETY: a property of a live view controller, on the main thread.
        let next: Option<Retained<AnyObject>> =
            unsafe { msg_send![&*controller, presentedViewController] };
        match next {
            Some(next) => controller = next,
            None => return controller,
        }
    }
}

/// An NSMutableArray of `items`.
fn array(items: &[Retained<AnyObject>]) -> Option<Retained<AnyObject>> {
    // SAFETY: a new array, and live objects added to it.
    unsafe {
        let array: Option<Retained<AnyObject>> = msg_send![class!(NSMutableArray), array];
        let array = array?;
        for item in items {
            let _: () = msg_send![&*array, addObject: &**item];
        }
        Some(array)
    }
}

/// Presents a picker for `items` (UTIs, or file URLs to export) in `mode`; its delegate hands the
/// pick to `picked`.
fn present(
    mtm: MainThreadMarker,
    items: &[Retained<AnyObject>],
    mode: isize,
    several: bool,
    picked: Option<Picked>,
) -> bool {
    let busy = PENDING.with(|p| p.borrow().is_some());
    if busy {
        return false;
    }
    let Some(window) = key_window() else {
        return false;
    };
    let Some(items) = array(items) else {
        return false;
    };
    // SAFETY: an instance of the class defined above, on the main thread.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    // SAFETY: main-thread UIKit calls with live objects. The initialisers are the iOS 13 ones
    // (deprecated from iOS 14, where they still work): no UniformTypeIdentifiers needed.
    unsafe {
        let root: Option<Retained<AnyObject>> = msg_send![&*window, rootViewController];
        let Some(root) = root else {
            return false;
        };
        let shown = topmost(root);
        let alloc: Allocated<AnyObject> = msg_send![class!(UIDocumentPickerViewController), alloc];
        let picker: Option<Retained<AnyObject>> = if mode == EXPORT_TO_SERVICE {
            msg_send![alloc, initWithURLs: &*items, inMode: mode]
        } else {
            msg_send![alloc, initWithDocumentTypes: &*items, inMode: mode]
        };
        let Some(picker) = picker else {
            return false;
        };
        let _: () = msg_send![&*picker, setDelegate: &*delegate];
        let _: () = msg_send![&*picker, setAllowsMultipleSelection: several];
        let completion: *const c_void = std::ptr::null();
        let _: () = msg_send![
            &*shown,
            presentViewController: &*picker,
            animated: true,
            completion: completion
        ];
    }
    PENDING.with(|p| *p.borrow_mut() = Some((delegate, picked)));
    true
}

/// Files' picker for `kind`; `picked` gets the paths (none when cancelled). False when no picker
/// could open: off the main thread, without a window, or with one already open.
pub fn pick(kind: Pick, picked: Picked) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let (uti, mode) = match kind {
        Pick::Files => ("public.item", IMPORT),
        Pick::Folder => ("public.folder", OPEN),
    };
    let uti: Retained<AnyObject> =
        Retained::into_super(Retained::into_super(NSString::from_str(uti)));
    present(mtm, &[uti], mode, kind == Pick::Files, Some(picked))
}

/// Files' sheet to save copies of `paths` where the player chooses. False when it could not open.
pub fn export(paths: &[PathBuf]) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let mut urls = Vec::new();
    for path in paths {
        let path = NSString::from_str(&path.to_string_lossy());
        // SAFETY: an NSURL class method with a live string.
        let url: Option<Retained<AnyObject>> =
            unsafe { msg_send![class!(NSURL), fileURLWithPath: &*path] };
        urls.extend(url);
    }
    !urls.is_empty() && present(mtm, &urls, EXPORT_TO_SERVICE, false, None)
}

/// The share sheet for the file at `path`: the apps that take it (SideStore, AltStore and
/// LiveContainer take an .ipa), and Save to Files. False when it could not open.
pub fn share(path: &std::path::Path) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let Some(window) = key_window() else {
        return false;
    };
    let path = NSString::from_str(&path.to_string_lossy());
    // SAFETY: main-thread UIKit and Foundation calls with live objects; the sheet is retained by
    // the controller that presents it.
    unsafe {
        let url: Option<Retained<AnyObject>> = msg_send![class!(NSURL), fileURLWithPath: &*path];
        let Some(url) = url else {
            return false;
        };
        let Some(items) = array(&[url]) else {
            return false;
        };
        let root: Option<Retained<AnyObject>> = msg_send![&*window, rootViewController];
        let Some(root) = root else {
            return false;
        };
        let shown = topmost(root);
        let alloc: Allocated<AnyObject> = msg_send![class!(UIActivityViewController), alloc];
        let none: *const AnyObject = std::ptr::null();
        let sheet: Option<Retained<AnyObject>> =
            msg_send![alloc, initWithActivityItems: &*items, applicationActivities: none];
        let Some(sheet) = sheet else {
            return false;
        };
        // An iPad shows it as a popover, which needs a place: the screen's middle.
        let popover: Option<Retained<AnyObject>> =
            msg_send![&*sheet, popoverPresentationController];
        let view: Option<Retained<AnyObject>> = msg_send![&*shown, view];
        if let (Some(popover), Some(view)) = (popover, view) {
            let bounds: super::Rect = msg_send![&*view, bounds];
            let middle = super::Rect {
                origin: super::Point {
                    x: bounds.origin.x + bounds.size.width / 2.0,
                    y: bounds.origin.y + bounds.size.height / 2.0,
                },
                size: super::Size {
                    width: 0.0,
                    height: 0.0,
                },
            };
            let _: () = msg_send![&*popover, setSourceView: &*view];
            let _: () = msg_send![&*popover, setSourceRect: middle];
        }
        let completion: *const c_void = std::ptr::null();
        let _: () = msg_send![
            &*shown,
            presentViewController: &*sheet,
            animated: true,
            completion: completion
        ];
    }
    true
}
