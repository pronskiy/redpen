//! Turns the hidden window into a non-activating floating NSPanel.
//!
//! Every use of `tauri-nspanel` lives in this file. It is a commit-pinned git dependency
//! and the risk register rates it Med/High, so the blast radius of swapping it — for a
//! Swift shim, or for hand-rolled objc2 — is one module.
//!
//! **Two independent focus paths have to be closed, and closing one is not enough:**
//!
//! 1. `canBecomeKeyWindow = false` stops the panel taking key focus once it is on screen.
//! 2. `NSWindowStyleMaskNonactivatingPanel` stops showing the panel *activating the app*
//!    in the first place.
//!
//! Miss (1) and the panel steals the caret. Miss (2) and the app comes forward, the menu
//! bar changes, and the user's typing goes somewhere else — even though the panel itself
//! never became key.

use tauri::{AppHandle, Manager, WebviewWindow, Wry};
use tauri_nspanel::{tauri_panel, ManagerExt, CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};

tauri_panel! {
    panel!(RedpenPanel {
        config: {
            can_become_key_window: false,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })
}

/// Convert **before the first show**. Converting a window that has already been shown costs
/// a frame of focus theft — the thing this whole epic exists to prevent (step B1.1).
pub fn convert(window: &WebviewWindow<Wry>) -> tauri::Result<()> {
    let panel = window.to_panel::<RedpenPanel>()?;

    // Above ordinary windows, below menus and status items.
    panel.set_level(PanelLevel::Floating.value());

    // Focus path 2, on a borderless mask.
    //
    // `StyleMask::new()` is Titled|Closable|Miniaturizable|Resizable — handing that to a
    // window the config created with `decorations: false` and `transparent: true` is a
    // contradiction AppKit resolves by aborting the process, and because it happens inside
    // an ObjC callback the only symptom is "panic in a function that cannot unwind" with
    // no backtrace. Borderless is also simply correct for a HUD card.
    panel.set_style_mask(StyleMask::empty().nonactivating_panel().value());

    // Follow the user across Spaces, and show over full-screen apps — redpen is summoned
    // wherever you happen to be typing, which is often a full-screen editor.
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .value(),
    );

    // C1.1: adaptive vibrancy. `Popover` follows the system light/dark appearance, unlike
    // `HudWindow` which is dark in both. Radius here rather than in CSS: with decorations
    // off the NSWindow itself needs the corner mask, or the blur renders as a hard square
    // behind rounded content.
    if let Err(e) = window_vibrancy::apply_vibrancy(
        window,
        window_vibrancy::NSVisualEffectMaterial::Popover,
        Some(window_vibrancy::NSVisualEffectState::Active),
        Some(12.0),
    ) {
        eprintln!("[redpen] vibrancy unavailable: {e}");
    }

    // Do not vanish when another app becomes active: the whole point is that the source
    // app keeps focus while the panel stays readable.
    panel.set_hides_on_deactivate(false);

    Ok(())
}

/// Show the panel without activating the app.
///
/// **Every one of these calls must happen on the main thread.** `tauri-nspanel` sends the
/// ObjC message straight through, unlike Tauri's own `window.show()`, which dispatches for
/// you. Calling it from the capture thread crashes the process outright:
///
/// ```text
/// asi: ["Must only be used from the main thread"]
/// -[NSWindow _doOrderWindow:] ... redpen_lib::panel::RawRedpenPanel
/// ```
///
/// So the dispatch lives here rather than at the call sites — the hazard belongs to the
/// module that owns the dependency, not to everyone who uses it.
/// Bumped once per completed show, so the capture thread can tell when the panel is
/// actually on screen rather than merely queued.
///
/// This matters because the *first* show activates the app — measurably: 12 ms of work the
/// first time, 0 ms every time after, and a `frontmost` that flips from the user's editor
/// to "redpen" one millisecond into the capture. Firing ⌘C before the show has landed means
/// racing that activation, and losing the race is the "nothing was copied" bug.
static SHOWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn shows_so_far() -> u64 {
    SHOWN.load(Ordering::Acquire)
}

/// Wait for a show requested after `seen` to complete.
pub fn wait_for_show(seen: u64, budget: std::time::Duration) {
    let deadline = std::time::Instant::now() + budget;
    while shows_so_far() == seen && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

pub fn show(app: &AppHandle) {
    let started = std::time::Instant::now();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        match handle.get_webview_panel(LABEL) {
            Ok(panel) => {
                panel.order_front_regardless();
                // Guardrail C1: hotkey → visible panel < 300 ms.
                println!("[redpen] panel visible in {} ms", started.elapsed().as_millis());
            }
            Err(_) => {
                if let Some(window) = handle.get_webview_window(LABEL) {
                    let _ = window.show();
                }
            }
        }
        SHOWN.fetch_add(1, Ordering::Release);
    });
}

pub fn hide(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(LABEL) {
            let _ = window.hide();
        }
    });
}

const LABEL: &str = "main";

// ---------------------------------------------------------------------------------------
// B1.3 — positioning
// ---------------------------------------------------------------------------------------

use objc2_app_kit::NSScreen;

/// Gap between the cursor and the panel, so the pointer never lands on top of the text.
const CURSOR_GAP: f64 = 14.0;

/// Put the panel near the mouse, clamped to the visible frame of whichever screen the mouse
/// is actually on.
///
/// **Everything here stays in AppKit screen coordinates** — bottom-left origin, y up, one
/// global space spanning every display. `NSEvent::mouseLocation`, `NSScreen::frame`,
/// `visibleFrame` and `setFrameOrigin` all speak it, so no conversion happens anywhere.
///
/// That is deliberate. The classic two-monitor "panel lands off-screen" bug comes from
/// mixing this with the top-left, y-down space that Tauri's `set_position` and the AX APIs
/// use: the flip needs the *primary* screen's height, and getting it from the wrong screen
/// puts the panel an entire display away. Never converting cannot get the conversion wrong.
pub fn position_at_mouse(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(window) = handle.get_webview_window(LABEL) else { return };
        let Ok(ptr) = window.ns_window() else { return };
        if ptr.is_null() {
            return;
        }
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };

        let mouse = NSEvent::mouseLocation();
        let size = ns.frame().size;

        // visibleFrame excludes the menu bar and the Dock, so a clamped panel is never
        // tucked under either.
        let visible = screen_for_point(mouse, mtm)
            .map(|s| s.visibleFrame())
            .unwrap_or_else(|| ns.frame());

        // Below and to the right of the cursor. In AppKit's y-up space "below" is a
        // *subtraction*, which is the sign error that puts the panel above the pointer.
        let mut x = mouse.x + CURSOR_GAP;
        let mut y = mouse.y - size.height - CURSOR_GAP;

        let max_x = visible.origin.x + visible.size.width - size.width;
        let max_y = visible.origin.y + visible.size.height - size.height;
        // clamp() panics if min > max, which happens when the panel is larger than the
        // screen — rare, but a crash is a poor way to find out.
        x = x.min(max_x).max(visible.origin.x);
        y = y.min(max_y).max(visible.origin.y);

        ns.setFrameOrigin(NSPoint::new(x, y));
    });
}

/// The screen the point falls on. Secondary displays sit at negative or offset coordinates
/// in the same global space, so this is a plain containment test — no per-screen maths.
fn screen_for_point(point: NSPoint, mtm: MainThreadMarker) -> Option<objc2::rc::Retained<NSScreen>> {
    let screens = NSScreen::screens(mtm);
    screens
        .iter()
        .find(|s| {
            let f = s.frame();
            point.x >= f.origin.x
                && point.x < f.origin.x + f.size.width
                && point.y >= f.origin.y
                && point.y < f.origin.y + f.size.height
        })
        .or_else(|| NSScreen::mainScreen(mtm))
}

// ---------------------------------------------------------------------------------------
// B1.4 — dismissal
// ---------------------------------------------------------------------------------------

use block2::RcBlock;
use objc2_app_kit::{NSApplicationActivationOptions, NSEventMask, NSRunningApplication, NSWorkspace};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicI32, Ordering};

/// Escape's virtual keycode. Layout-independent, like the copy keycode in `capture.rs`.
const KEYCODE_ESCAPE: u16 = 53;

// ---------------------------------------------------------------------------------------
// Focus hand-back
// ---------------------------------------------------------------------------------------

/// The app that owned focus before redpen did, so we can give it back.
///
/// **Why this exists.** Capture synthesises ⌘C, which macOS delivers to whatever app is
/// *active* — not to whatever window is on top. If redpen is the active app, the ⌘C lands
/// on our own panel, which has no selection: nothing reaches the pasteboard and capture
/// times out reporting "nothing was copied (secure input, or no selection)". Diagnosed from
/// a trace where the one failing press read `frontmost="redpen"` and five successful ones
/// read `frontmost="PhpStorm-EAP"`.
///
/// The whole of Epic B stops the panel *taking* focus, and it works — showing the panel is
/// not what activates us. But two ordinary things still do, and neither goes through the
/// panel at all:
///
/// 1. **Opening the tray menu.** Clicking a status item activates its app.
/// 2. **Launching.** Every `tauri dev` rebuild restarts the app active.
///
/// Either way we stay active until the user clicks another app — so the *next* hotkey press
/// fails, and the one after it works because the failure made the user click back into
/// their editor. That is the "only the first press fails" report.
static PREVIOUS_APP: AtomicI32 = AtomicI32::new(0);

/// A hand-back we performed ourselves, and the moment we asked for it. `Some((0, _))` means
/// "whichever app macOS picks".
///
/// Without this the fix would defeat itself: handing focus back posts
/// `NSWorkspaceDidActivateApplication`, which the dismissal observer reads as "the user has
/// gone somewhere else" and hides the panel. The panel would flash and vanish on precisely
/// the presses this is meant to repair. The window is short so that a *real* app switch a
/// moment later still dismisses normally.
static HANDBACK: std::sync::Mutex<Option<(i32, std::time::Instant)>> = std::sync::Mutex::new(None);

const HANDBACK_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// True if this activation is the one we just asked for, and consumes it.
fn is_our_handback(activated_pid: Option<i32>) -> bool {
    let mut slot = match HANDBACK.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    match *slot {
        Some((pid, at)) if at.elapsed() < HANDBACK_GRACE && (pid == 0 || Some(pid) == activated_pid) => {
            *slot = None;
            true
        }
        Some((_, at)) if at.elapsed() >= HANDBACK_GRACE => {
            *slot = None;
            false
        }
        _ => false,
    }
}

fn expect_handback(pid: i32) {
    if let Ok(mut slot) = HANDBACK.lock() {
        *slot = Some((pid, std::time::Instant::now()));
    }
}

fn our_pid() -> i32 {
    NSRunningApplication::currentApplication().processIdentifier()
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(
        option: u32,
        relative_to: u32,
    ) -> *mut objc2_foundation::NSArray<objc2_foundation::NSDictionary>;
}

/// `kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements`, and the window
/// list is returned strictly front-to-back.
const WINDOW_LIST_OPTIONS: u32 = (1 << 0) | (1 << 4);

/// Whoever owns the frontmost ordinary window that is not ours — the app the user was
/// looking at, and whose selection they mean.
///
/// This is ground truth rather than history, which is the whole point: `PREVIOUS_APP` only
/// knows about app switches it *observed*, and the case that actually bites has none to
/// observe. redpen comes up active on launch (every `tauri dev` rebuild), the user never
/// clicks away because their selection is already made, and so the first press finds an
/// empty memory and copies from our own panel.
///
/// Layer 0 is the ordinary window layer. Filtering on it skips the menu bar, the Dock, and
/// status items — and would skip our own panel too, which sits at the floating level, though
/// this runs before the panel is shown anyway.
fn app_behind_us() -> Option<i32> {
    use objc2_foundation::{NSNumber, NSString};

    let ours = our_pid();
    let windows = unsafe {
        let ptr = CGWindowListCopyWindowInfo(WINDOW_LIST_OPTIONS, 0);
        objc2::rc::Retained::from_raw(ptr)?
    };

    let layer_key = NSString::from_str("kCGWindowLayer");
    let pid_key = NSString::from_str("kCGWindowOwnerPID");

    for window in windows.iter() {
        let number = |key: &NSString| {
            window
                .objectForKey(key)
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .map(|n| n.as_i64())
        };
        if number(&layer_key) != Some(0) {
            continue;
        }
        match number(&pid_key) {
            Some(pid) if pid as i32 != ours && pid > 0 => return Some(pid as i32),
            _ => continue,
        }
    }
    None
}

/// Is redpen the active application — i.e. would a synthetic ⌘C land on our own panel?
pub fn is_frontmost() -> bool {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|a| a.processIdentifier() == our_pid())
        .unwrap_or(false)
}

/// Hand focus back to the app the user was actually typing in.
///
/// Note this only ever *restores* activation, never steals it: it does nothing unless we
/// are already wrongly active, so it cannot violate the Epic B guarantee.
///
/// `ActivateIgnoringOtherApps` is deliberately not used — it is deprecated and inert from
/// macOS 14 on. Plain activation is what works now.
pub fn yield_focus_to_source(app: &AppHandle) {
    let _ = app.run_on_main_thread(move || {
        if !is_frontmost() {
            return;
        }
        // Window order first, remembered app second. The order matters: the memory is empty
        // in exactly the case that fails.
        let target = app_behind_us().or_else(|| match PREVIOUS_APP.load(Ordering::Relaxed) {
            pid if pid > 0 => Some(pid),
            _ => None,
        });

        if let Some(pid) = target {
            if let Some(previous) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
                expect_handback(pid);
                let name = previous.localizedName().map(|n| n.to_string()).unwrap_or_default();
                let ok = previous.activateWithOptions(NSApplicationActivationOptions::empty());
                println!("[redpen] handing focus back to {name:?} (pid {pid}) · accepted={ok}");
                if ok {
                    return;
                }
            }
        }
        // Last resort: relinquish and let macOS pick the successor. Weaker than activating a
        // named app — we cannot say which pid to expect — but it beats copying from our own
        // panel.
        println!("[redpen] no app to hand focus back to; deactivating");
        expect_handback(0);
        objc2_app_kit::NSApplication::sharedApplication(
            objc2_foundation::MainThreadMarker::new().expect("run_on_main_thread"),
        )
        .deactivate();
    });
}

/// Block until we are no longer the active app. Called off the hotkey thread, just before
/// the ⌘C goes out, because `yield_focus_to_source` is asynchronous — the activation is a
/// round trip through the window server, and firing ⌘C before it lands would copy from the
/// panel anyway.
pub fn wait_until_not_frontmost(budget: std::time::Duration) {
    let deadline = std::time::Instant::now() + budget;
    while is_frontmost() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Install the three dismissal paths.
///
/// A non-activating panel can never become key, so **there is no keydown to listen for in
/// the webview** — the JS listener that worked in A2.3 is now permanently silent, because
/// Esc goes to whatever app the user is actually typing in. Observing the events at the
/// system level is the only option left, which is exactly why the spec flags this as the
/// subtle step of the epic.
///
/// A *global* monitor observes without consuming: Esc still reaches the source app, and it
/// also dismisses the panel. That is the behaviour we want — swallowing Esc system-wide
/// while the panel is up would break vim, dialogs and everything else.
pub fn install_dismissal<F>(app: &AppHandle, on_dismiss: F)
where
    F: Fn() + Clone + 'static,
{
    let handle = app.clone();
    let visible = move || {
        handle
            .get_webview_window(LABEL)
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false)
    };

    // --- Esc, from any app -------------------------------------------------------------
    let (v, d) = (visible.clone(), on_dismiss.clone());
    let key_block = RcBlock::new(move |event: NonNull<NSEvent>| {
        let event = unsafe { event.as_ref() };
        if event.keyCode() == KEYCODE_ESCAPE && v() {
            d();
        }
    });
    let global_keys =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &key_block);

    // --- Esc, when the panel itself somehow has focus -----------------------------------
    let (v, d) = (visible.clone(), on_dismiss.clone());
    let local_block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let ev = unsafe { event.as_ref() };
        if ev.keyCode() == KEYCODE_ESCAPE && v() {
            d();
            return std::ptr::null_mut(); // consume it here; we are the intended target
        }
        event.as_ptr()
    });
    // Unlike the global variant, this one can swallow the event, so it is unsafe.
    let local_keys = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &local_block)
    };

    // --- Click anywhere outside ---------------------------------------------------------
    // Global monitors only see events bound for *other* apps, so any click this fires on is
    // by definition outside the panel. No hit-testing needed.
    let (v, d) = (visible.clone(), on_dismiss.clone());
    let click_block = RcBlock::new(move |_event: NonNull<NSEvent>| {
        if v() {
            d();
        }
    });
    let clicks = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown
            .union(NSEventMask::RightMouseDown)
            .union(NSEventMask::OtherMouseDown),
        &click_block,
    );

    // --- Frontmost app changed (⌘Tab) ---------------------------------------------------
    // Showing the panel activates nothing, so this only fires on a real switch. The guard
    // is for the case where clicking the panel activates *us* — hiding on our own
    // activation would make the panel dismiss itself.
    let (v, d) = (visible.clone(), on_dismiss.clone());
    let our_pid = NSRunningApplication::currentApplication().processIdentifier();
    let switch_block = RcBlock::new(move |notification: NonNull<NSNotification>| {
        let note = unsafe { notification.as_ref() };
        let activated_pid = unsafe {
            note.userInfo()
                .and_then(|info| {
                    info.objectForKey(&*objc2_foundation::NSString::from_str("NSWorkspaceApplicationKey"))
                })
                .map(|app| {
                    let running: &NSRunningApplication = &*(Retained::as_ptr(&app) as *const NSRunningApplication);
                    running.processIdentifier()
                })
        };
        // Anyone but us is a candidate to hand focus back to later.
        if let Some(pid) = activated_pid {
            if pid != our_pid {
                PREVIOUS_APP.store(pid, Ordering::Relaxed);
            }
        }
        // Our own hand-back is not the user walking away, so it must not dismiss.
        if is_our_handback(activated_pid) {
            return;
        }
        if activated_pid != Some(our_pid) && v() {
            d();
        }
    });
    let workspace_observer = unsafe {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        center.addObserverForName_object_queue_usingBlock(
            Some(&objc2_foundation::NSString::from_str("NSWorkspaceDidActivateApplicationNotification")),
            None,
            None,
            &switch_block,
        )
    };

    // Seed it, or the very first press after launch has nobody to hand focus back to —
    // and that press is exactly the one that fails.
    if let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() {
        let pid = front.processIdentifier();
        if pid != our_pid {
            PREVIOUS_APP.store(pid, Ordering::Relaxed);
        }
    }

    // These must outlive the call or the monitors deregister immediately. They are meant to
    // live for the whole process, so leaking them is the correct lifetime, not a shortcut.
    std::mem::forget((global_keys, local_keys, clicks, workspace_observer));
    std::mem::forget((key_block, local_block, click_block, switch_block));
}
