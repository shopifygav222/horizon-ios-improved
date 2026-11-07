//! Controllers on iPhone and iPad: Apple's GameController framework, which pairs and reads every pad iOS
//! supports (DualSense, DualShock 4, Xbox, MFi), and the other UIKit services the game needs. The game's
//! libraries forbid `unsafe`, so the Objective-C calls live here and the game sees plain values. GameController keeps its state current on the main thread's run
//! loop, which is the one winit runs, so reading a pad is reading properties: no event queue to drain.
//! Elsewhere the functions report no controllers.

/// One controller iOS has paired.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Controller {
    /// The vendor's name for it, when it gives one.
    pub name: Option<String>,
    /// iOS's product category: "DualSense", "DualShock 4", "Xbox One", "Switch Pro Controller"...
    pub category: String,
    /// It has the full gamepad profile (two sticks, four face buttons, shoulders and triggers).
    pub gamepad: bool,
}

/// The active pad's inputs: sticks -1..1 with up positive, triggers 0..1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub category: String,
    pub a: bool,
    pub b: bool,
    pub x: bool,
    pub y: bool,
    pub left_shoulder: bool,
    pub right_shoulder: bool,
    /// The left one of the middle buttons (View, Share, Create, minus); not every pad has it.
    pub options: bool,
    /// The right one (Menu, Options, plus).
    pub menu: bool,
    pub left_stick_button: bool,
    pub right_stick_button: bool,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    pub triggers: [f32; 2],
}

#[cfg(target_os = "ios")]
mod ios {
    use super::{Controller, Snapshot};
    use objc2_game_controller::{
        GCController, GCControllerButtonInput, GCDevice, GCExtendedGamepad,
    };

    pub fn controllers() -> Vec<Controller> {
        // SAFETY: class and property reads of GameController objects, which iOS keeps alive while listed.
        unsafe {
            GCController::controllers()
                .iter()
                .map(|c| Controller {
                    name: c.vendorName().map(|n| n.to_string()),
                    category: c.productCategory().to_string(),
                    gamepad: c.extendedGamepad().is_some(),
                })
                .collect()
        }
    }

    fn pressed(button: Option<objc2::rc::Retained<GCControllerButtonInput>>) -> bool {
        // SAFETY: a property read of a live button.
        button.is_some_and(|b| unsafe { b.isPressed() })
    }

    pub fn active() -> Option<Snapshot> {
        // SAFETY: as above; every object is retained for the length of the read.
        unsafe {
            let gamepad = |c: &GCController| c.extendedGamepad().is_some();
            let controller = GCController::current()
                .filter(|c| gamepad(c))
                .or_else(|| GCController::controllers().iter().find(|c| gamepad(c)))?;
            let pad: objc2::rc::Retained<GCExtendedGamepad> = controller.extendedGamepad()?;
            let dpad = pad.dpad();
            let (left, right) = (pad.leftThumbstick(), pad.rightThumbstick());
            Some(Snapshot {
                category: controller.productCategory().to_string(),
                a: pressed(Some(pad.buttonA())),
                b: pressed(Some(pad.buttonB())),
                x: pressed(Some(pad.buttonX())),
                y: pressed(Some(pad.buttonY())),
                left_shoulder: pressed(Some(pad.leftShoulder())),
                right_shoulder: pressed(Some(pad.rightShoulder())),
                options: pressed(pad.buttonOptions()),
                menu: pressed(Some(pad.buttonMenu())),
                left_stick_button: pressed(pad.leftThumbstickButton()),
                right_stick_button: pressed(pad.rightThumbstickButton()),
                dpad_up: pressed(Some(dpad.up())),
                dpad_down: pressed(Some(dpad.down())),
                dpad_left: pressed(Some(dpad.left())),
                dpad_right: pressed(Some(dpad.right())),
                left_stick: [left.xAxis().value(), left.yAxis().value()],
                right_stick: [right.xAxis().value(), right.yAxis().value()],
                triggers: [pad.leftTrigger().value(), pad.rightTrigger().value()],
            })
        }
    }
}

/// Every controller iOS has paired.
pub fn controllers() -> Vec<Controller> {
    #[cfg(target_os = "ios")]
    return ios::controllers();
    #[cfg(not(target_os = "ios"))]
    Vec::new()
}

/// The pad that drives the game, read now: the one iOS last saw used, else the first with a full gamepad
/// profile. `None` when no such pad is connected.
pub fn active() -> Option<Snapshot> {
    #[cfg(target_os = "ios")]
    return ios::active();
    #[cfg(not(target_os = "ios"))]
    None
}

/// Whether an external keyboard is currently attached (main-thread UIKit input use only).
pub fn keyboard_connected() -> bool {
    #[cfg(target_os = "ios")]
    // SAFETY: a read-only GameController class property, on the game's main thread.
    return unsafe { objc2_game_controller::GCKeyboard::coalescedKeyboard().is_some() };
    #[cfg(not(target_os = "ios"))]
    false
}

/// Whether a hardware mouse is currently attached (main-thread UIKit input use only).
pub fn mouse_connected() -> bool {
    #[cfg(target_os = "ios")]
    // SAFETY: a read-only GameController class property, on the game's main thread.
    return unsafe { !objc2_game_controller::GCMouse::mice().is_empty() };
    #[cfg(not(target_os = "ios"))]
    false
}

/// Read iPadOS/iOS text when a game edit shortcut explicitly requests paste.
/// `None` means the caller is not on UIKit's main thread; no pasteboard access is attempted.
pub fn clipboard_text() -> Option<String> {
    #[cfg(target_os = "ios")]
    {
        let _main = objc2::MainThreadMarker::new()?;
        let board = objc2_ui_kit::UIPasteboard::generalPasteboard();
        // SAFETY: UIKit is accessed on its main thread and the retained NSString
        // is converted to owned text before returning to the caller.
        Some(unsafe { board.string() }.map_or_else(String::new, |s| s.to_string()))
    }
    #[cfg(not(target_os = "ios"))]
    None
}

// ---- haptic feedback ----

// `initWithStyle:` is deprecated from iOS 17.5 in favour of a view-bound generator; the app supports
// iOS 16, where only it exists.

/// A light tap — button presses.
#[allow(deprecated)]
pub fn haptic_light() {
    #[cfg(target_os = "ios")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        use objc2_ui_kit::{UIImpactFeedbackGenerator, UIImpactFeedbackStyle};
        let feedback = UIImpactFeedbackGenerator::initWithStyle(
            mtm.alloc(),
            UIImpactFeedbackStyle::Light,
        );
        feedback.impactOccurred();
    }
}

/// A medium tap — toggle activations.
#[allow(deprecated)]
pub fn haptic_medium() {
    #[cfg(target_os = "ios")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        use objc2_ui_kit::{UIImpactFeedbackGenerator, UIImpactFeedbackStyle};
        let feedback = UIImpactFeedbackGenerator::initWithStyle(
            mtm.alloc(),
            UIImpactFeedbackStyle::Medium,
        );
        feedback.impactOccurred();
    }
}

/// A rigid tap — joystick forward lock engaging.
#[allow(deprecated)]
pub fn haptic_rigid() {
    #[cfg(target_os = "ios")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        use objc2_ui_kit::{UIImpactFeedbackGenerator, UIImpactFeedbackStyle};
        let feedback = UIImpactFeedbackGenerator::initWithStyle(
            mtm.alloc(),
            UIImpactFeedbackStyle::Rigid,
        );
        feedback.impactOccurred();
    }
}

/// A selection tick — hotbar slide.
pub fn haptic_selection() {
    #[cfg(target_os = "ios")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        use objc2_ui_kit::UISelectionFeedbackGenerator;
        let feedback = UISelectionFeedbackGenerator::new(mtm);
        feedback.selectionChanged();
    }
}

/// Write text copied by the game to the iOS system pasteboard. False off UIKit's main thread.
pub fn set_clipboard_text(text: &str) -> bool {
    #[cfg(target_os = "ios")]
    if let Some(_main) = objc2::MainThreadMarker::new() {
        let board = objc2_ui_kit::UIPasteboard::generalPasteboard();
        let text = objc2_foundation::NSString::from_str(text);
        // SAFETY: main-thread UIKit access; the string stays alive throughout
        // the call and UIPasteboard copies its value.
        unsafe { board.setString(Some(&text)) };
        return true;
    }
    let _ = text;
    false
}

// ---- the rest of UIKit the game needs: the screen, the keyboard, URLs, Photos, motion, Keychain ----

#[cfg(target_os = "ios")]
mod uikit {
    use objc2::{
        MainThreadMarker, class,
        encode::{Encode, Encoding},
        msg_send,
        rc::Retained,
        runtime::AnyObject,
    };
    use objc2_foundation::NSString;
    use std::ffi::c_void;

    // Full CoreGraphics layouts, though only some fields are read.
    #[allow(dead_code)]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    // SAFETY: CoreGraphics' CGPoint, two CGFloats (doubles on every 64-bit Apple target).
    unsafe impl Encode for Point {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    #[allow(dead_code)]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }
    // SAFETY: CoreGraphics' CGSize.
    unsafe impl Encode for Size {
        const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }
    // SAFETY: CoreGraphics' CGRect, a CGPoint then a CGSize.
    unsafe impl Encode for Rect {
        const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RotationRate {
        x: f64,
        y: f64,
        z: f64,
    }
    // SAFETY: CoreMotion's CMRotationRate, an anonymous struct of three doubles.
    unsafe impl Encode for RotationRate {
        const ENCODING: Encoding =
            Encoding::Struct("?", &[f64::ENCODING, f64::ENCODING, f64::ENCODING]);
    }

    fn application() -> Option<Retained<AnyObject>> {
        MainThreadMarker::new()?;
        // SAFETY: a UIKit class property, on the main thread.
        unsafe { msg_send![class!(UIApplication), sharedApplication] }
    }

    fn key_window() -> Option<Retained<AnyObject>> {
        let app = application()?;
        // SAFETY: a property of the live application object, on the main thread.
        unsafe { msg_send![&*app, keyWindow] }
    }

    /// winit's view: the key window's root view controller's view.
    fn root_view() -> Option<Retained<AnyObject>> {
        let window = key_window()?;
        // SAFETY: properties of the live key window and its controller, on the main thread.
        unsafe {
            let controller: Option<Retained<AnyObject>> = msg_send![&*window, rootViewController];
            let controller = controller?;
            msg_send![&*controller, view]
        }
    }

    pub fn screen_scale() -> Option<f64> {
        MainThreadMarker::new()?;
        // SAFETY: UIScreen class and property reads, on the main thread.
        unsafe {
            let screen: Option<Retained<AnyObject>> = msg_send![class!(UIScreen), mainScreen];
            let screen = screen?;
            let scale: f64 = msg_send![&*screen, scale];
            Some(scale)
        }
    }

    pub fn interface_rotation() -> Option<i32> {
        let window = key_window()?;
        // SAFETY: properties of the live key window and its scene, on the main thread.
        unsafe {
            let scene: Option<Retained<AnyObject>> = msg_send![&*window, windowScene];
            let scene = scene?;
            let orientation: isize = msg_send![&*scene, interfaceOrientation];
            Some(match orientation {
                3 => 1, // landscape right: the top of the device to the left
                2 => 2, // portrait upside down
                4 => 3, // landscape left
                _ => 0,
            })
        }
    }

    pub fn keyboard_top() -> Option<u32> {
        let view = root_view()?;
        // SAFETY: properties of the live view and of its keyboard layout guide (iOS 15+).
        unsafe {
            let guide: Option<Retained<AnyObject>> = msg_send![&*view, keyboardLayoutGuide];
            let guide = guide?;
            let frame: Rect = msg_send![&*guide, layoutFrame];
            let bounds: Rect = msg_send![&*view, bounds];
            let scale: f64 = msg_send![&*view, contentScaleFactor];
            // Closed (or floating), the guide is only the bottom safe-area inset.
            if frame.size.height < 60.0 || frame.origin.y >= bounds.size.height {
                return None;
            }
            Some((frame.origin.y.max(0.0) * scale).round() as u32)
        }
    }

    pub fn open_url(url: &str) -> bool {
        let Some(app) = application() else {
            return false;
        };
        let text = NSString::from_str(url);
        // SAFETY: main-thread UIKit calls with live objects; openURL accepts a nil completion.
        unsafe {
            let url: Option<Retained<AnyObject>> = msg_send![class!(NSURL), URLWithString: &*text];
            let Some(url) = url else {
                return false;
            };
            let options: Option<Retained<AnyObject>> = msg_send![class!(NSDictionary), dictionary];
            let Some(options) = options else {
                return false;
            };
            let completion: *const c_void = std::ptr::null();
            let _: () = msg_send![
                &*app,
                openURL: &*url,
                options: &*options,
                completionHandler: completion
            ];
        }
        true
    }

    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" {
        fn UIImageWriteToSavedPhotosAlbum(
            image: *const c_void,
            target: *const c_void,
            selector: *const c_void,
            context: *mut c_void,
        );
    }

    pub fn save_to_photos(path: &str) -> bool {
        if MainThreadMarker::new().is_none() {
            return false;
        }
        let path = NSString::from_str(path);
        // SAFETY: main thread; UIKit retains the image until it has been written.
        unsafe {
            let image: Option<Retained<AnyObject>> =
                msg_send![class!(UIImage), imageWithContentsOfFile: &*path];
            let Some(image) = image else {
                return false;
            };
            UIImageWriteToSavedPhotosAlbum(
                Retained::as_ptr(&image).cast(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
        }
        true
    }

    pub fn power() -> Option<(bool, isize)> {
        // SAFETY: NSProcessInfo is thread-safe; plain property reads.
        unsafe {
            let info: Option<Retained<AnyObject>> = msg_send![class!(NSProcessInfo), processInfo];
            let info = info?;
            let low: bool = msg_send![&*info, isLowPowerModeEnabled];
            let thermal: isize = msg_send![&*info, thermalState];
            Some((low, thermal))
        }
    }

    #[link(name = "CoreMotion", kind = "framework")]
    unsafe extern "C" {}

    thread_local! {
        static MOTION: std::cell::RefCell<Option<Retained<AnyObject>>> =
            const { std::cell::RefCell::new(None) };
    }

    pub fn gyro_start() -> bool {
        MOTION.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                return true;
            }
            // SAFETY: one CMMotionManager, created, configured and read on this thread.
            unsafe {
                let manager: Option<Retained<AnyObject>> = msg_send![class!(CMMotionManager), new];
                let Some(manager) = manager else {
                    return false;
                };
                let available: bool = msg_send![&*manager, isGyroAvailable];
                if !available {
                    return false;
                }
                let _: () = msg_send![&*manager, setGyroUpdateInterval: 0.01f64];
                let _: () = msg_send![&*manager, startGyroUpdates];
                *slot = Some(manager);
            }
            true
        })
    }

    pub fn gyro_stop() {
        if let Some(manager) = MOTION.with(|slot| slot.borrow_mut().take()) {
            // SAFETY: the manager this thread started.
            let _: () = unsafe { msg_send![&*manager, stopGyroUpdates] };
        }
    }

    pub fn gyro_sample() -> Option<([f32; 3], f64)> {
        MOTION.with(|slot| {
            let slot = slot.borrow();
            let manager = slot.as_ref()?;
            // SAFETY: property reads of the live manager and its latest sample.
            unsafe {
                let data: Option<Retained<AnyObject>> = msg_send![&**manager, gyroData];
                let data = data?;
                let rate: RotationRate = msg_send![&*data, rotationRate];
                let time: f64 = msg_send![&*data, timestamp];
                Some(([rate.x as f32, rate.y as f32, rate.z as f32], time))
            }
        })
    }

    type CfRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: CfRef;
        static kCFTypeDictionaryKeyCallBacks: u8;
        static kCFTypeDictionaryValueCallBacks: u8;
        fn CFDictionaryCreate(
            allocator: CfRef,
            keys: *const CfRef,
            values: *const CfRef,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CfRef;
        fn CFDataCreate(allocator: CfRef, bytes: *const u8, length: isize) -> CfRef;
        fn CFDataGetBytePtr(data: CfRef) -> *const u8;
        fn CFDataGetLength(data: CfRef) -> isize;
        fn CFStringCreateWithBytes(
            allocator: CfRef,
            bytes: *const u8,
            length: isize,
            encoding: u32,
            external: u8,
        ) -> CfRef;
        fn CFRelease(object: CfRef);
    }

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecClass: CfRef;
        static kSecClassGenericPassword: CfRef;
        static kSecAttrService: CfRef;
        static kSecAttrAccount: CfRef;
        static kSecAttrAccessible: CfRef;
        static kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly: CfRef;
        static kSecValueData: CfRef;
        static kSecReturnData: CfRef;
        static kSecMatchLimit: CfRef;
        static kSecMatchLimitOne: CfRef;
        fn SecItemCopyMatching(query: CfRef, result: *mut CfRef) -> i32;
        fn SecItemAdd(attributes: CfRef, result: *mut CfRef) -> i32;
        fn SecItemUpdate(query: CfRef, attributes: CfRef) -> i32;
        fn SecItemDelete(query: CfRef) -> i32;
    }

    const NOT_FOUND: i32 = -25300;
    const UTF8: u32 = 0x0800_0100;

    /// A CoreFoundation object this code created, released when dropped.
    struct Owned(CfRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: created by a CF Create function, released once.
                unsafe { CFRelease(self.0) }
            }
        }
    }

    fn string(text: &str) -> Owned {
        // SAFETY: `text` is valid UTF-8 for its length; CoreFoundation copies it.
        Owned(unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                text.as_ptr(),
                text.len() as isize,
                UTF8,
                0,
            )
        })
    }

    fn dictionary(pairs: &[(CfRef, CfRef)]) -> Owned {
        let keys: Vec<CfRef> = pairs.iter().map(|p| p.0).collect();
        let values: Vec<CfRef> = pairs.iter().map(|p| p.1).collect();
        // SAFETY: both arrays hold `pairs.len()` live CF objects, which the CFType callbacks retain.
        Owned(unsafe {
            CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                pairs.len() as isize,
                (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
                (&raw const kCFTypeDictionaryValueCallBacks).cast(),
            )
        })
    }

    pub fn keychain_get(service: &str, account: &str) -> Result<Option<Vec<u8>>, i32> {
        let (service, account) = (string(service), string(account));
        // SAFETY: the query holds live CF objects; a match is a CFData this code then owns.
        unsafe {
            let query = dictionary(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
                (kSecReturnData, kCFBooleanTrue),
                (kSecMatchLimit, kSecMatchLimitOne),
            ]);
            let mut result: CfRef = std::ptr::null();
            match SecItemCopyMatching(query.0, &mut result) {
                0 => {
                    let data = Owned(result);
                    let length = CFDataGetLength(data.0).max(0) as usize;
                    if length == 0 {
                        return Ok(Some(Vec::new()));
                    }
                    Ok(Some(
                        std::slice::from_raw_parts(CFDataGetBytePtr(data.0), length).to_vec(),
                    ))
                }
                NOT_FOUND => Ok(None),
                status => Err(status),
            }
        }
    }

    pub fn keychain_set(service: &str, account: &str, secret: &[u8]) -> Result<(), i32> {
        let (service, account) = (string(service), string(account));
        // SAFETY: every dictionary holds live CF objects; Security copies what it keeps.
        unsafe {
            let data = Owned(CFDataCreate(
                std::ptr::null(),
                secret.as_ptr(),
                secret.len() as isize,
            ));
            let query = dictionary(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
            ]);
            let update = dictionary(&[
                (kSecValueData, data.0),
                (
                    kSecAttrAccessible,
                    kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
                ),
            ]);
            match SecItemUpdate(query.0, update.0) {
                0 => Ok(()),
                NOT_FOUND => {
                    let item = dictionary(&[
                        (kSecClass, kSecClassGenericPassword),
                        (kSecAttrService, service.0),
                        (kSecAttrAccount, account.0),
                        (kSecValueData, data.0),
                        (
                            kSecAttrAccessible,
                            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
                        ),
                    ]);
                    match SecItemAdd(item.0, std::ptr::null_mut()) {
                        0 => Ok(()),
                        status => Err(status),
                    }
                }
                status => Err(status),
            }
        }
    }

    pub fn keychain_delete(service: &str, account: &str) -> Result<(), i32> {
        let (service, account) = (string(service), string(account));
        // SAFETY: the query holds live CF objects.
        unsafe {
            let query = dictionary(&[
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
            ]);
            match SecItemDelete(query.0) {
                0 | NOT_FOUND => Ok(()),
                status => Err(status),
            }
        }
    }
}

/// UIKit's screen scale: points to pixels at full resolution. `None` off the main thread or iOS.
pub fn screen_scale() -> Option<f64> {
    #[cfg(target_os = "ios")]
    return uikit::screen_scale();
    #[cfg(not(target_os = "ios"))]
    None
}

/// The interface's rotation from portrait, as Android's `Display.getRotation` counts it (0..3).
pub fn interface_rotation() -> i32 {
    #[cfg(target_os = "ios")]
    return uikit::interface_rotation().unwrap_or(0);
    #[cfg(not(target_os = "ios"))]
    0
}

/// The software keyboard's top edge in the game view's pixels, or `None` while it is closed.
pub fn keyboard_top() -> Option<u32> {
    #[cfg(target_os = "ios")]
    return uikit::keyboard_top();
    #[cfg(not(target_os = "ios"))]
    None
}

/// Open `url` in the app iOS chooses for it (Safari for a web page, Files for `shareddocuments:`).
pub fn open_url(url: &str) -> bool {
    #[cfg(target_os = "ios")]
    return uikit::open_url(url);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = url;
        false
    }
}

/// Add the image at `path` to the photo library (asking for permission the first time).
pub fn save_to_photos(path: &str) -> bool {
    #[cfg(target_os = "ios")]
    return uikit::save_to_photos(path);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = path;
        false
    }
}

/// Low Power Mode and `NSProcessInfo.thermalState` (0 nominal to 3 critical).
pub fn power() -> Option<(bool, isize)> {
    #[cfg(target_os = "ios")]
    return uikit::power();
    #[cfg(not(target_os = "ios"))]
    None
}

/// Start CoreMotion's gyroscope; false when the device has none.
pub fn gyro_start() -> bool {
    #[cfg(target_os = "ios")]
    return uikit::gyro_start();
    #[cfg(not(target_os = "ios"))]
    false
}

pub fn gyro_stop() {
    #[cfg(target_os = "ios")]
    uikit::gyro_stop();
}

/// The latest rotation rate (rad/s, device axes) and its timestamp in seconds.
pub fn gyro_sample() -> Option<([f32; 3], f64)> {
    #[cfg(target_os = "ios")]
    return uikit::gyro_sample();
    #[cfg(not(target_os = "ios"))]
    None
}

/// A generic password item in the Keychain, readable on this device after its first unlock.
/// Errors are Security's OSStatus codes.
pub fn keychain_get(service: &str, account: &str) -> Result<Option<Vec<u8>>, i32> {
    #[cfg(target_os = "ios")]
    return uikit::keychain_get(service, account);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (service, account);
        Err(-4)
    }
}

pub fn keychain_set(service: &str, account: &str, secret: &[u8]) -> Result<(), i32> {
    #[cfg(target_os = "ios")]
    return uikit::keychain_set(service, account, secret);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (service, account, secret);
        Err(-4)
    }
}

pub fn keychain_delete(service: &str, account: &str) -> Result<(), i32> {
    #[cfg(target_os = "ios")]
    return uikit::keychain_delete(service, account);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (service, account);
        Err(-4)
    }
}
