//! Where is the selected text on screen? Epic D: the card goes beside the selection, not at
//! the mouse.
//!
//! The answer comes from the app itself through Accessibility, the API VoiceOver uses — no
//! screenshots. What each app answers was measured by `scripts/ax-probe.swift` (SPEC D1):
//!
//! * TextEdit, Safari, Chrome: the rect of the selected words (`AXBoundsForRange`, or text
//!   markers for web content).
//! * Slack: only the message box's frame. VS Code: a 0-wide hidden input Monaco keeps at the
//!   caret. Both are the focused element's frame, the second tier.
//! * Ghostty, Telegram: nothing useful. The card stays at the mouse.
//!
//! Chromium and Electron build their accessibility tree only when asked, and take a second or
//! two to do it, so `wake` sets `AXManualAccessibility` on every app as it is activated —
//! by the time a hotkey is pressed the tree is there.
//!
//! The geometry is pure and unit-tested; only `mac` touches AppKit or Accessibility.

/// A rect in AppKit screen coordinates: bottom-left origin, y up, one space across every
/// display — the space `panel.rs` works in (decision #29).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn top(&self) -> f64 {
        self.y + self.h
    }
}

/// Accessibility speaks top-left, y-down coordinates. Flipping needs the **primary**
/// screen's height, never the height of the screen the rect is on: the classic two-monitor
/// bug that throws the card a whole display away (decision #29).
pub fn from_ax(x: f64, y: f64, w: f64, h: f64, primary_height: f64) -> Rect {
    Rect { x, y: primary_height - y - h, w, h }
}

/// Which kind of answer the app gave, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Selection,
    Field,
}

/// A selection rect is believable if it has a size, is no taller than a paragraph, and sits
/// inside the app's window. Zero rects are how Slack and Chromium page text say "no idea".
pub fn believable_selection(r: Rect, window: Option<Rect>) -> bool {
    (r.w > 0.0 || r.h > 0.0) && r.h <= 400.0 && inside(r, window)
}

/// A field frame is only worth anchoring to when it is small enough to say where the text
/// is: Slack's message box (35 pt) or VS Code's caret input (18 pt, zero wide) yes; Ghostty's
/// whole terminal area no. Over 200 pt tall, or nearly the window itself, means the mouse is
/// the better guess.
pub fn useful_field(r: Rect, window: Option<Rect>) -> bool {
    if r.h <= 0.0 || r.h > 200.0 || !inside(r, window) {
        return false;
    }
    match window {
        Some(w) => r.w * r.h < 0.5 * w.w * w.h,
        None => true,
    }
}

fn inside(r: Rect, window: Option<Rect>) -> bool {
    let Some(w) = window else { return true };
    let slack = 2.0;
    r.x >= w.x - slack && r.y >= w.y - slack && r.x + r.w <= w.x + w.w + slack && r.top() <= w.top() + slack
}

/// Gap between the selection and the card.
pub const GAP: f64 = 8.0;

/// Where the card's origin (its bottom-left, in AppKit terms) goes for a given anchor.
///
/// Below the selection, left edges aligned, so the line you wrote stays visible above the
/// card the way an inline hint sits. If there is not room below on that screen, above it.
/// Then clamped to the visible frame, like the mouse placement. Decision #49.
pub fn place(anchor: Rect, card_w: f64, card_h: f64, visible: Rect) -> (f64, f64) {
    let below = anchor.y - GAP - card_h;
    let above = anchor.top() + GAP;
    let room_below = below >= visible.y;
    let room_above = above + card_h <= visible.top();
    let y = if room_below || !room_above {
        // Neither side fits: keep it below and let the clamp pull it up. Covering part of
        // a tall selection beats leaving the screen.
        if room_below { below } else if anchor.y - visible.y >= visible.top() - anchor.top() { below } else { above }
    } else {
        above
    };
    let x = anchor.x;
    let max_x = visible.x + visible.w - card_w;
    let max_y = visible.top() - card_h;
    // min/max, never clamp(): clamp panics when the card is larger than the screen.
    (x.min(max_x).max(visible.x), y.min(max_y).max(visible.y))
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{believable_selection, from_ax, useful_field, Rect, Tier};
    use objc2_application_services::{AXError, AXUIElement, AXValue, AXValueType};
    use objc2_core_foundation::{CFArray, CFBoolean, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};
    use std::ptr::NonNull;
    use std::time::{Duration, Instant};

    /// The whole lookup's budget. The card shows in 3 ms today against a 300 ms guardrail,
    /// and every app measured answered in under 15 ms. Decision #49.
    pub const BUDGET: Duration = Duration::from_millis(50);

    fn attr(el: &AXUIElement, name: &str) -> Option<CFRetained<CFType>> {
        let key = CFString::from_str(name);
        let mut out: *const CFType = std::ptr::null();
        let err = unsafe { el.copy_attribute_value(&key, NonNull::from(&mut out)) };
        if err != AXError::Success {
            return None;
        }
        // Copy rule: we own the reference.
        NonNull::new(out as *mut CFType).map(|p| unsafe { CFRetained::from_raw(p) })
    }

    fn param(el: &AXUIElement, name: &str, arg: &CFType) -> Option<CFRetained<CFType>> {
        let key = CFString::from_str(name);
        let mut out: *const CFType = std::ptr::null();
        let err = unsafe { el.copy_parameterized_attribute_value(&key, arg, NonNull::from(&mut out)) };
        if err != AXError::Success {
            return None;
        }
        NonNull::new(out as *mut CFType).map(|p| unsafe { CFRetained::from_raw(p) })
    }

    fn element(v: &CFType) -> Option<CFRetained<AXUIElement>> {
        v.downcast_ref::<AXUIElement>().map(CFRetained::from)
    }

    fn value<T: Default>(v: &CFType, ty: AXValueType) -> Option<T> {
        let v = v.downcast_ref::<AXValue>()?;
        let mut out = T::default();
        unsafe { v.value(ty, NonNull::from(&mut out).cast()) }.then_some(out)
    }

    /// The element's own frame, AX coordinates.
    fn frame(el: &AXUIElement) -> Option<CGRect> {
        let p: CGPoint = value(&*attr(el, "AXPosition")?, AXValueType::CGPoint)?;
        let s: CGSize = value(&*attr(el, "AXSize")?, AXValueType::CGSize)?;
        Some(CGRect { origin: p, size: s })
    }

    fn to_rect(r: CGRect, primary_height: f64) -> Rect {
        from_ax(r.origin.x, r.origin.y, r.size.width, r.size.height, primary_height)
    }

    fn primary_height() -> f64 {
        // Main-thread-only API is avoided on purpose: this runs on whatever thread the caller
        // is on. CGDisplayBounds of the main display is the primary screen's frame.
        extern "C" {
            fn CGMainDisplayID() -> u32;
            fn CGDisplayBounds(display: u32) -> CGRect;
        }
        unsafe { CGDisplayBounds(CGMainDisplayID()) }.size.height
    }

    fn focused_app() -> Option<CFRetained<AXUIElement>> {
        let system = unsafe { AXUIElement::new_system_wide() };
        unsafe { system.set_messaging_timeout(BUDGET.as_secs_f32()) };
        let app = element(&*attr(&system, "AXFocusedApplication")?)?;
        Some(app)
    }

    /// Breadth-first through the focused window for the element that says it has focus.
    /// Electron can answer per element while saying "no value" for the app's focus.
    fn walk_for_focus(window: &AXUIElement, deadline: Instant) -> Option<CFRetained<AXUIElement>> {
        let mut queue: Vec<CFRetained<AXUIElement>> = vec![CFRetained::from(window)];
        let mut i = 0;
        while i < queue.len() && i < 500 && Instant::now() < deadline {
            let el = queue[i].clone();
            i += 1;
            if i > 1 {
                if let Some(f) = attr(&el, "AXFocused") {
                    if f.downcast_ref::<CFBoolean>().map(|b| b.as_bool()) == Some(true) {
                        return Some(el);
                    }
                }
            }
            if let Some(kids) = attr(&el, "AXChildren") {
                if let Some(arr) = kids.downcast_ref::<CFArray>() {
                    for k in 0..arr.count() {
                        let ptr = unsafe { arr.value_at_index(k) } as *const CFType;
                        if let Some(child) = unsafe { ptr.as_ref() }.and_then(element) {
                            queue.push(child);
                        }
                    }
                }
            }
        }
        None
    }

    /// The best anchor the frontmost app gives, within `BUDGET`, or `None` for "use the mouse".
    pub fn selection_anchor() -> Option<(Rect, Tier)> {
        let deadline = Instant::now() + BUDGET;
        let app = focused_app()?;
        unsafe { app.set_messaging_timeout(BUDGET.as_secs_f32()) };
        let ph = primary_height();

        let window_el = attr(&app, "AXFocusedWindow").and_then(|w| element(&w));
        let window = window_el.as_deref().and_then(frame).map(|r| to_rect(r, ph));

        let focused = match attr(&app, "AXFocusedUIElement").and_then(|f| element(&f)) {
            Some(f) => f,
            None => walk_for_focus(window_el.as_deref()?, deadline)?,
        };
        if Instant::now() >= deadline {
            return None;
        }

        // Tier 1: the selection itself — a text control, then web text markers.
        let selection = attr(&focused, "AXSelectedTextRange")
            .and_then(|r| param(&focused, "AXBoundsForRange", &r))
            .and_then(|v| value::<CGRect>(&v, AXValueType::CGRect))
            .map(|r| to_rect(r, ph))
            .filter(|r| believable_selection(*r, window))
            .or_else(|| {
                attr(&focused, "AXSelectedTextMarkerRange")
                    .and_then(|m| param(&focused, "AXBoundsForTextMarkerRange", &m))
                    .and_then(|v| value::<CGRect>(&v, AXValueType::CGRect))
                    .map(|r| to_rect(r, ph))
                    .filter(|r| believable_selection(*r, window))
            });
        if let Some(r) = selection {
            return Some((r, Tier::Selection));
        }
        if Instant::now() >= deadline {
            return None;
        }

        // Tier 2: the focused element's frame, if it is small enough to mean something.
        frame(&focused).map(|r| to_rect(r, ph)).filter(|r| useful_field(*r, window)).map(|r| (r, Tier::Field))
    }

    /// Chromium browsers ignore Electron's switch and only build their tree for
    /// `AXEnhancedUserInterface`. That attribute is the one with a reputation (it can make
    /// native apps animate window moves oddly), so it is set for these and nothing else.
    const CHROMIUM: &[&str] = &[
        "com.google.Chrome",
        "com.google.Chrome.canary",
        "org.chromium.Chromium",
        "com.brave.Browser",
        "com.microsoft.edgemac",
        "company.thebrowser.Browser",
        "com.vivaldi.Vivaldi",
        "com.operasoftware.Opera",
    ];

    /// Ask an app to build its accessibility tree now. `AXManualAccessibility` is Electron's
    /// and a no-op anywhere else, so it is set on every app as it activates.
    pub fn wake(pid: i32) {
        let app = unsafe { AXUIElement::new_application(pid) };
        let chromium = objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .and_then(|a| a.bundleIdentifier())
            .is_some_and(|id| CHROMIUM.contains(&id.to_string().as_str()));
        unsafe {
            app.set_messaging_timeout(0.1);
            let _ = app.set_attribute_value(&CFString::from_static_str("AXManualAccessibility"), CFBoolean::new(true));
            if chromium {
                let _ = app.set_attribute_value(&CFString::from_static_str("AXEnhancedUserInterface"), CFBoolean::new(true));
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{selection_anchor, wake};

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 1512.0, h: 920.0 };
    const CARD_W: f64 = 480.0;
    const CARD_H: f64 = 340.0;

    #[test]
    fn flipping_uses_the_primary_height() {
        // A 14 pt line whose top is 100 pt below the top of a 982 pt primary screen.
        let r = from_ax(293.0, 100.0, 95.0, 14.0, 982.0);
        assert_eq!(r, Rect { x: 293.0, y: 868.0, w: 95.0, h: 14.0 });
    }

    #[test]
    fn the_card_goes_below_the_selection_with_left_edges_aligned() {
        let sel = Rect { x: 300.0, y: 700.0, w: 95.0, h: 14.0 };
        let (x, y) = place(sel, CARD_W, CARD_H, SCREEN);
        assert_eq!(x, 300.0);
        assert_eq!(y + CARD_H, 700.0 - GAP, "the card's top sits GAP under the selection");
    }

    #[test]
    fn no_room_below_puts_it_above() {
        let sel = Rect { x: 300.0, y: 120.0, w: 95.0, h: 14.0 };
        let (_, y) = place(sel, CARD_W, CARD_H, SCREEN);
        assert_eq!(y, 134.0 + GAP, "the card's bottom sits GAP over the selection");
    }

    #[test]
    fn a_selection_at_the_right_edge_keeps_the_card_on_screen() {
        let sel = Rect { x: 1400.0, y: 700.0, w: 95.0, h: 14.0 };
        let (x, _) = place(sel, CARD_W, CARD_H, SCREEN);
        assert_eq!(x, 1512.0 - CARD_W);
    }

    #[test]
    fn a_selection_taller_than_the_room_on_either_side_still_lands_on_screen() {
        let sel = Rect { x: 100.0, y: 200.0, w: 600.0, h: 600.0 };
        let (x, y) = place(sel, CARD_W, CARD_H, SCREEN);
        assert!(x >= 0.0 && x + CARD_W <= SCREEN.w);
        assert!(y >= 0.0 && y + CARD_H <= SCREEN.h);
    }

    #[test]
    fn a_secondary_display_to_the_left_has_negative_coordinates() {
        let left = Rect { x: -1920.0, y: 0.0, w: 1920.0, h: 1055.0 };
        let sel = Rect { x: -1500.0, y: 800.0, w: 120.0, h: 16.0 };
        let (x, y) = place(sel, CARD_W, CARD_H, left);
        assert_eq!(x, -1500.0);
        assert_eq!(y, 800.0 - GAP - CARD_H);
    }

    #[test]
    fn a_zero_rect_is_not_a_selection() {
        assert!(!believable_selection(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, None));
    }

    #[test]
    fn a_selection_outside_the_window_is_not_believed() {
        let window = Some(Rect { x: 100.0, y: 100.0, w: 600.0, h: 400.0 });
        assert!(believable_selection(Rect { x: 150.0, y: 300.0, w: 90.0, h: 14.0 }, window));
        assert!(!believable_selection(Rect { x: 900.0, y: 300.0, w: 90.0, h: 14.0 }, window));
    }

    #[test]
    fn slacks_message_box_and_vs_codes_caret_are_useful_fields() {
        let slack_window = Some(Rect { x: 0.0, y: 0.0, w: 1512.0, h: 949.0 });
        assert!(useful_field(Rect { x: 371.0, y: 61.0, w: 1119.0, h: 35.0 }, slack_window));
        let code_window = Some(Rect { x: 15.0, y: 41.0, w: 1440.0, h: 900.0 });
        assert!(useful_field(Rect { x: 130.0, y: 855.0, w: 0.0, h: 18.0 }, code_window), "a 0-wide caret still marks a place");
    }

    #[test]
    fn ghosttys_whole_terminal_is_not_a_useful_field() {
        let window = Some(Rect { x: 185.0, y: 168.0, w: 1078.0, h: 781.0 });
        assert!(!useful_field(Rect { x: 185.0, y: 168.0, w: 1078.0, h: 713.0 }, window));
    }
}
