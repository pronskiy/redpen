// Used by scripts/translate-guardrails.py to test under the Russian layout.
// inputsource [id-substring]: prints the current keyboard layout, or selects the first one whose id contains the argument.
import Carbon
let list = TISCreateInputSourceList([kTISPropertyInputSourceCategory: kTISCategoryKeyboardInputSource] as CFDictionary, false).takeRetainedValue() as! [TISInputSource]
func id(_ s: TISInputSource) -> String { Unmanaged<CFString>.fromOpaque(TISGetInputSourceProperty(s, kTISPropertyInputSourceID)).takeUnretainedValue() as String }
if CommandLine.arguments.count > 1 {
    guard let s = list.first(where: { id($0).contains(CommandLine.arguments[1]) }) else { print("no such source"); exit(1) }
    TISSelectInputSource(s)
}
print(id(TISCopyCurrentKeyboardInputSource().takeRetainedValue()))
