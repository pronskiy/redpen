// Epic D, step D1.1: what does the frontmost app report, through Accessibility, about where
// the selected text is on screen? Touches no app code; the answer decides whether Epic D
// gets built (SPEC D1 guardrails).
//
//   swiftc -O scripts/ax-probe.swift -o /tmp/ax-probe
//   /tmp/ax-probe [--fix] [--draw] [--delay N]
//
//   --fix     first set AXManualAccessibility (Electron) and AXEnhancedUserInterface
//             (Chromium) on the app, which make them build their accessibility tree
//   --draw    outline each reported rect on screen for 1.5 s (red: selection, blue: field),
//             the only real test of whether a rect is right
//   --delay N wait N seconds first, so you can switch to the app and select something
//   --reset   set both switches back to false and exit; they otherwise stay on until the
//             app quits
//
// Prints one JSON object. Rects are in AppKit coordinates (bottom-left origin, y up), the
// space panel.rs works in, converted with the *primary* screen's height (decision #29).

import Cocoa
import ApplicationServices

let args = CommandLine.arguments
let fix = args.contains("--fix")
let draw = args.contains("--draw")
if let i = args.firstIndex(of: "--delay"), i + 1 < args.count, let d = Int(args[i + 1]) {
    for left in stride(from: d, to: 0, by: -1) {
        FileHandle.standardError.write("switch to the app… \(left)\n".data(using: .utf8)!)
        Thread.sleep(forTimeInterval: 1)
    }
}

guard AXIsProcessTrusted() else {
    print(#"{"error":"no Accessibility permission for whatever runs this"}"#)
    exit(2)
}
// Not NSWorkspace.frontmostApplication: a command-line tool has no run loop to receive its
// updates, so after --delay it still names whatever was frontmost at launch (the terminal).
// The system-wide element's focused application is asked fresh every time.
let systemWide = AXUIElementCreateSystemWide()
var focusedApp: AnyObject?
var pid: pid_t = 0
if AXUIElementCopyAttributeValue(systemWide, kAXFocusedApplicationAttribute as CFString, &focusedApp) == .success {
    AXUIElementGetPid(focusedApp as! AXUIElement, &pid)
}
guard pid != 0, let front = NSRunningApplication(processIdentifier: pid) else {
    print(#"{"error":"no focused application"}"#)
    exit(1)
}
let app = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(app, 0.2)

if args.contains("--reset") {
    AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanFalse)
    AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanFalse)
    exit(0)
}

var out: [String: Any] = ["app": front.localizedName ?? "?", "bundle": front.bundleIdentifier ?? "?"]
func ms(_ t0: CFAbsoluteTime) -> Int { Int((CFAbsoluteTimeGetCurrent() - t0) * 1000) }

if fix {
    let t0 = CFAbsoluteTimeGetCurrent()
    let a = AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    let b = AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    out["fix"] = ["manual": a.rawValue, "enhanced": b.rawValue, "ms": ms(t0)]
    Thread.sleep(forTimeInterval: 0.5)  // let the app build its tree
}

func get(_ el: AXUIElement, _ name: String) -> AnyObject? {
    var v: AnyObject?
    return AXUIElementCopyAttributeValue(el, name as CFString, &v) == .success ? v : nil
}
func param(_ el: AXUIElement, _ name: String, _ arg: AnyObject) -> AnyObject? {
    var v: AnyObject?
    return AXUIElementCopyParameterizedAttributeValue(el, name as CFString, arg, &v) == .success ? v : nil
}
/// A zero rect is how several apps (Slack, Chromium page text) say "no idea", so it counts
/// as no answer and the next tier gets its turn.
func rect(_ v: AnyObject?) -> CGRect? {
    guard let v, CFGetTypeID(v) == AXValueGetTypeID() else { return nil }
    var r = CGRect.zero
    guard AXValueGetValue(v as! AXValue, .cgRect, &r), r.width > 0 || r.height > 0 else { return nil }
    return r
}
func frame(_ el: AXUIElement) -> CGRect? {
    var p = CGPoint.zero, s = CGSize.zero
    guard let pv = get(el, kAXPositionAttribute), let sv = get(el, kAXSizeAttribute),
          AXValueGetValue(pv as! AXValue, .cgPoint, &p), AXValueGetValue(sv as! AXValue, .cgSize, &s)
    else { return nil }
    return CGRect(origin: p, size: s)
}

// AX: top-left origin, y down. AppKit: bottom-left, y up. Flip with the primary screen only.
let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
func appKit(_ r: CGRect) -> CGRect {
    CGRect(x: r.origin.x, y: primaryHeight - r.origin.y - r.height, width: r.width, height: r.height)
}
func json(_ r: CGRect) -> [String: Int] {
    let a = appKit(r)
    return ["x": Int(a.origin.x), "y": Int(a.origin.y), "w": Int(a.width), "h": Int(a.height)]
}

// A rect is plausible if it has a size, looks like a line or a few of text, and sits inside
// the app's focused window. Plausible is not correct — --draw is how you see correct.
var window: CGRect?
if let w = get(app, kAXFocusedWindowAttribute) { window = frame(w as! AXUIElement) }
func plausible(_ r: CGRect, maxHeight: CGFloat) -> Bool {
    guard r.width > 0, r.height >= 6, r.height <= maxHeight else { return false }
    guard let w = window else { return true }
    return w.insetBy(dx: -2, dy: -2).contains(r)
}

var t0 = CFAbsoluteTimeGetCurrent()
// Ask the app first, then the system-wide element: some apps (Electron) answer only one.
var focusedAny: AnyObject?
var err = AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute as CFString, &focusedAny)
out["focused_err_app"] = err.rawValue
if focusedAny == nil {
    let system = AXUIElementCreateSystemWide()
    AXUIElementSetMessagingTimeout(system, 0.2)
    err = AXUIElementCopyAttributeValue(system, kAXFocusedUIElementAttribute as CFString, &focusedAny)
    out["focused_err_system"] = err.rawValue
}
// Last resort: walk the focused window for the element that says it has focus. Electron
// apps can answer per element while still returning "no value" for the app's focus.
if focusedAny == nil, let w = get(app, kAXFocusedWindowAttribute) {
    var queue: [AXUIElement] = [w as! AXUIElement]
    var seen = 0
    while !queue.isEmpty && seen < 4000 {
        let el = queue.removeFirst(); seen += 1
        if (get(el, kAXFocusedAttribute) as? Bool) == true, get(el, kAXRoleAttribute) as? String != kAXWindowRole {
            focusedAny = el; out["focused_via"] = "tree walk"; break
        }
        if let kids = get(el, kAXChildrenAttribute) as? [AXUIElement] { queue.append(contentsOf: kids) }
    }
    out["walked"] = seen
}
out["focused_ms"] = ms(t0)
var selection: CGRect?
var field: CGRect?

if let f = focusedAny {
    let el = f as! AXUIElement
    out["role"] = get(el, kAXRoleAttribute) as? String ?? "?"
    if let text = get(el, kAXSelectedTextAttribute) as? String {
        out["selected_text"] = String(text.prefix(60))
    }

    // Tier 1a: a plain text control. AXSelectedTextRange → AXBoundsForRange.
    t0 = CFAbsoluteTimeGetCurrent()
    if let range = get(el, kAXSelectedTextRangeAttribute),
       let r = rect(param(el, kAXBoundsForRangeParameterizedAttribute, range)) {
        selection = r
        out["selection_via"] = "AXBoundsForRange"
    }
    // Tier 1b: web content (WebKit, Chromium) speaks text markers instead.
    if selection == nil, let marker = get(el, "AXSelectedTextMarkerRange"),
       let r = rect(param(el, "AXBoundsForTextMarkerRange", marker)) {
        selection = r
        out["selection_via"] = "AXBoundsForTextMarkerRange"
    }
    out["selection_ms"] = ms(t0)

    // Tier 2: the focused element's own frame.
    t0 = CFAbsoluteTimeGetCurrent()
    field = frame(el)
    out["field_ms"] = ms(t0)
} else {
    out["role"] = NSNull()
}

if let s = selection {
    out["selection"] = json(s)
    out["selection_plausible"] = plausible(s, maxHeight: 400)
}
if let f = field {
    out["field"] = json(f)
    out["field_plausible"] = plausible(f, maxHeight: 4000)
}
if let w = window { out["window"] = json(w) }
let mouse = NSEvent.mouseLocation
out["mouse"] = ["x": Int(mouse.x), "y": Int(mouse.y)]

let data = try! JSONSerialization.data(withJSONObject: out, options: [.sortedKeys])
print(String(data: data, encoding: .utf8)!)

if draw {
    let nsApp = NSApplication.shared
    nsApp.setActivationPolicy(.accessory)
    var windows: [NSWindow] = []
    for (r, color) in [(selection, NSColor.systemRed), (field, NSColor.systemBlue)] {
        guard let r else { continue }
        let w = NSWindow(contentRect: appKit(r).insetBy(dx: -3, dy: -3), styleMask: .borderless, backing: .buffered, defer: false)
        w.isOpaque = false
        w.backgroundColor = .clear
        w.level = .screenSaver
        w.ignoresMouseEvents = true
        let v = NSView(frame: w.contentView!.bounds)
        v.wantsLayer = true
        v.layer?.borderColor = color.cgColor
        v.layer?.borderWidth = 2
        w.contentView = v
        w.orderFrontRegardless()
        windows.append(w)
    }
    RunLoop.current.run(until: Date().addingTimeInterval(1.5))
}
