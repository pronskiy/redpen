# Manual test matrix — capture (SPEC step A1.4)

`capture.rs` cannot be tested in CI: it needs a real pasteboard, real Accessibility
permission, and a real app to steal a selection from. The unit tests in `capture.rs` cover
the *algorithm* against a mock. **This file covers reality, and it is the test artifact.**

Run it with the app up (`npm run tauri dev`) and the log tailing:

```sh
tail -f /tmp/redpen-dev.log | grep --line-buffered redpen
```

Select text, press ⌥⌘E, and record what the log printed.

> **Re-run this whole file after every Tauri or plugin bump**, and any time macOS revokes
> Accessibility permission — which it does on rebuild (see README).

Date run: **2026-08-26** · Build: `ec2e33c` (dev) · Run by: **Roman**

## 1. Capture matrix — guardrail A1

Does a selection get captured at all, and does the text come back intact?

| App | Where exactly | Captured? | Text intact? | Notes |
|-----|---------------|-----------|--------------|-------|
| Slack | a message in a channel | ✅ | 🔲 | Electron — the case AX capture fails on, decision #4 |
| Chrome | a GitHub comment textarea | ✅ | 🔲 | |
| Telegram | a chat message | ✅ | 🔲 | |
| Mail.app | a draft body | ✅ | 🔲 | native AppKit control |
| VS Code | an open editor buffer | ✅ | 🔲 | Electron again, different text model |

Worth also noting: multi-line selections, text with emoji, and text with curly quotes —
the corpus is full of all three. Fixtures are in `docs/capture-fixtures/`: open one in any
editor, select all, press ⌥⌘E. The log prints a character count, so compare it against
`wc -m` on the file.

| Edge case | Result | Notes |
|-----------|--------|-------|
| Multi-line selection | ✅ | newlines preserved? |
| Emoji / non-ASCII | ✅ | char count in the log should look sane |
| Very long selection (~5k chars) | ✅ | any lag? |
| Nothing selected | ✅ | expect a clean `nothing was copied` |

**Keyboard layout matters, and the unit tests cannot see it.** `send_copy()` is injected as a
closure in the capture tests, so the keystroke itself is never exercised there — a layout bug
passes every test and fails every real use (decision #27).

| Layout active when pressing ⌥⌘E | Result | Notes |
|---------------------------------|--------|-------|
| U.S. / Latin | 🔲 | |
| Russian (ЙЦУКЕН) | 🔲 | the case that produced ⌘A "select all" before the keycode fix |
| Any other non-Latin layout you use | 🔲 | |

## 2. Clipboard integrity — guardrail A1

The truce with clipboard managers. Method: copy a canary, capture something else, paste.

Run `scripts/clipboard-check.sh [text|image|rtf]` — it seeds a canary, waits while you
capture, then tells you whether the canary survived.

| Check | Result | Notes |
|-------|--------|-------|
| Plain text clipboard survives a capture | ✅ | `scripts/clipboard-check.sh text` |
| **Image** clipboard survives a capture | ✅ | `scripts/clipboard-check.sh image` — an image seeds 9 pasteboard types, all restored |
| Rich text / RTF survives | ✅ | `scripts/clipboard-check.sh rtf` |
| No fight with Raycast clipboard history | ✅ | history should not fill with captured selections |
| No fight with Maccy | ✅ | if installed |

If a clipboard manager *is* running, the expected behaviour is that redpen **skips** the
restore rather than fighting it — `changeCount != before + 1`. A clipboard that does *not*
come back with a manager running is not necessarily a bug; check the history app first.

## 3. Secure input — guardrail A1

macOS blocks synthetic keystrokes while secure input is active. This must fail *fast and
politely*, never hang.

| Check | Result | Notes |
|-------|--------|-------|
| In a password field, ⌥⌘E returns within ~2s | ✅ | expect `nothing was copied (secure input, or no selection)` |
| No hang, no beachball | ✅ | capture runs off the hotkey thread |
| No crash | ✅ | |
| App still works normally afterwards | ✅ | press ⌥⌘E on real text again |

## 4. Permission behaviour

| Check | Result | Notes |
|-------|--------|-------|
| Without Accessibility permission, the error names the fix | ✅ | verified 2026-08-26: `could not send ⌘C: the application does not have the permission to simulate input (check Accessibility permission)` |
| After granting, capture works without a restart | ✅ | verified 2026-08-26 |
| After a rebuild, permission state | ✅ | expected to reset; record what actually happens |

## 5. Updates — tray "Check for Updates…"

`tauri-plugin-updater` cannot be tested in CI either: it needs a real signed bundle, a real
release endpoint, and a real macOS Accessibility grant to lose. The check path can be
exercised now; the install path cannot be trusted until there is a Developer ID.

The endpoint is `update_endpoint` in `config.json` and hot-reloads, so each row below is
just an edit and a click on the tray item — no rebuild.

| Check | Result | Notes |
|-------|--------|-------|
| `update_endpoint` empty → panel says checks are off | 🔲 | must not look like a network error |
| Garbage URL (`"not a url"`) → panel names it | 🔲 | parse failure, not a hang |
| Unreachable host → panel shows the failure | 🔲 | should not sit on "Checking…" forever |
| Valid endpoint, same version → "You're up to date" | 🔲 | names the running version |
| Valid endpoint, newer version → "Update available" | 🔲 | version, notes, Install button |
| Unsigned build → Accessibility warning shown | 🔲 | expected in dev; this is the whole point of the warning |
| Panel dismisses with Esc mid-check | 🔲 | no crash, no orphaned request |

**The install path is the one that can quietly break capture.** After the first real
update, re-run §1 (capture matrix) and §4 (permission behaviour) before trusting it:

| Check | Result | Notes |
|-------|--------|-------|
| Developer ID signed build → no warning shown | 🔲 | needs a $99/yr identity; blocked until then |
| Install and restart replaces the bundle | 🔲 | |
| Accessibility grant survives the update | 🔲 | **the acceptance test.** Fails on any unsigned build, by design |
| ⌥⌘E still captures after the update | 🔲 | if this fails, the grant was revoked |

## Outcome

Fill this in before advancing to Phase A2, and copy the verdict into the
**Exit guardrails — Phase A1 → A2** table in `SPEC.md`.

- Capture matrix: **5 / 5 apps**
- Clipboard integrity: **pass** (text, image, RTF; no fight with clipboard history)
- Secure input: **pass** — clean timeout, no hang, no crash
- Blocking problems found: **none**

Guardrail A1 → A2 is green. Recorded in `SPEC.md`.
