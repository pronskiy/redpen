# redpen — Technical Spec

**Author:** Roman Pronskiy · **Created:** 2026-08-26

> 📄 **This is a living document.** Status markers, decisions, and guardrail outcomes are meant to be updated as the work happens. See [How to Update This Document](#how-to-update-this-document) before editing.

### Changelog

| Date | Change | Author |
|------|--------|--------|
| 2026-08-26 | Initial spec created | Roman |
| 2026-08-26 | Added §1 Positioning vs Apple Intelligence; decision #13; sherlocking risk | Roman |
| 2026-08-26 | Reordered Epic A to A3 → A1 → A2; A3.1/A3.2 delivered; decisions #14–19 | Roman |

### Status legend

🔲 Not started · 🔄 In progress · ✅ Done · ⏸️ Blocked · ❌ Cut

### Current focus

**Now on:** Epic C → Phase C1 → step C1.1 — vibrancy and the card design. **Needs Roman's design call first.**

**Epic A is complete.** Hotkey → capture → streamed critique → Esc aborts, end to end. Two
loose ends carried forward:

1. **The A3 usefulness guardrail is still unrated** — the only guardrail in Epic A with
   nothing behind it, and the one the risk register calls fatal.
2. **Latency has 4 samples, not 10**, and one exceeded the bar. Ten presses closes it.

Note for B1.4: Esc currently works through a JS `keydown` listener because the window is
still an ordinary focusable window. Once it becomes a non-activating panel it will never be
key, Esc will land in the source app, and that listener stops firing — it must be replaced
with a global event monitor, not patched.

**Still open: the A3 usefulness guardrail.** Two prompt versions have been run and the
structure half passes 20/20, but nothing is rated, so the kill criterion has never been
evaluated. Decision #14 put that gate first precisely so it would be answered before this
much existed. It is now the only guardrail in Epic A with nothing behind it.

A1.1 was built ahead of the gate (it is a scaffold, not an investment — the kill criterion
stays executable). Nothing further in A1/A2 should be built until A3 passes.

**Needs Roman, and blocks everything else.** A3.1 (prompt) and A3.2 (harness) are done; the
gate is waiting on the corpus. Start here: `docs/corpus/README.md`.

---

## 1. Executive summary

redpen is a macOS menu-bar app for a non-native English writer who wants to *improve*, not just get corrected. Select text anywhere, hit a global hotkey, and a translucent floating callout appears with an LLM critique: what sounds off, why, and how a native speaker would phrase it. Unlike every existing tool in this space (Apple Writing Tools, WritingTools, RewriteBar, Grammarly), redpen **never touches your text** — you read the critique and retype the fix yourself, which is what builds the skill. The system prompt knows the author's L1 is Russian and hunts for calques specifically.

### Positioning vs Apple Intelligence Writing Tools

As of macOS Tahoe (26.x), Apple ships system-wide Writing Tools: Proofread with per-change explanations, Rewrite (incl. a custom "describe your change" prompt), popup-near-text UX. **The mechanics of redpen are commodity — Apple gives them away for free.** redpen is justified only by what Apple structurally won't build:

1. **Opposite flow direction.** Apple's explanations justify an "apply" button; the flow ends in replaced text. redpen's flow ends in the user retyping — pedagogy over output. Apple will never ship a deliberately "inconvenient" tool.
2. **Naturalness, not correctness.** Proofread targets errors; grammatically-correct-but-non-native phrasing ("I have a possibility to…") passes clean. Rewrite fixes it silently, without the *why*. The L1-aware critique lives exactly in that gap.
3. **Memory.** Apple explains a change once and forgets. The error journal (Epic E) — "14th article miss this month" — is the moat; Apple optimizes output quality for a billion users, not pedagogy for language learners.
4. **Control.** Own model, own prompt, own L1 context.

Consequence for priorities: the panel (Epics B–C) carries no defensible value by itself; critique quality (A3 gate) and the journal (Epic E) carry all of it.

---

## 2. Technical decisions

| Area | Decision | Rationale |
|------|----------|-----------|
| Shell | Tauri v2 (Rust + system WKWebView) | UI is the differentiator; iterating a translucent card is faster in CSS than SwiftUI. Native surface is small and fully covered by plugins |
| Text capture | Simulated ⌘C via `enigo` + pasteboard snapshot/restore | Works everywhere ⌘C works, incl. Electron. AX-based capture is patchy exactly in the apps that matter (Slack, browsers). Technique proven by WritingTools |
| Floating panel | `tauri-nspanel`, **pinned to a commit** | Only way to get a non-activating NSPanel in Tauri. Git-branch dependency → pin. Both focus-stealing paths must be disabled: `canBecomeKeyWindow` override AND the nonactivating style mask |
| LLM | Anthropic Messages API only, SSE streaming, configurable `base_url` | One provider keeps v1 small; custom base_url gives proxies/compatible endpoints for free. Multi-provider is explicitly out of scope for v1 |
| Config | Strict JSON at `~/Library/Application Support/redpen/config.json`, hot-reloaded; the system prompt is a separate Markdown file referenced by `system_prompt_path` | The system prompt gets edited dozens of times a day during tuning; an editor beats any settings UI — and an escaped 141-line JSON string would beat neither. Annotated copy: `config.example.jsonc`, parsed by nothing |
| Response contract | Markdown critique + trailing ```json block with error tags | Journal (Epic E) needs structured tags from day one even though the journal ships later. Tags at the *end* so the user reads prose, not streaming JSON |
| Core principle | **Never insert, replace, or paste anything** | The entire thesis. No paste-back code path exists in this codebase |
| Caret positioning | Deferred to Epic D via a Swift shim (`swift-rs`) | Raw AX C API from Rust is ~120 lines of unsafe FFI; a `@_cdecl` Swift function is 15 lines. Panel positions at the mouse cursor until then |
| Model | `claude-sonnet-5`, adaptive thinking, `output_config.effort: "medium"` | Decision #25 (supersedes #15). Effort — not model tier — is the latency lever; thinking is on by default and renders as a blank card until it finishes (decision #16). This tier does not accept `fallbacks` (decision #24) |
| Tag vocabulary | Fixed 20-tag list, embedded in the prompt itself | Epic E cannot aggregate freeform tags. The harness extracts the list from the prompt and validates every response against it, so there is one source of truth |
| License hygiene | Read WritingTools for techniques; copy zero lines | WritingTools is GPL-3.0; repo visibility here is undecided, so keep the codebase unencumbered |

---

## 3. Architecture overview

```
        global hotkey (tauri-plugin-global-shortcut)
                          │
                          ▼
  ┌─────────────────────────────────────────────┐
  │ capture.rs                                  │
  │ pasteboard snapshot → ⌘C via enigo →        │
  │ poll changeCount (5ms, 2s timeout) →        │
  │ read text → restore snapshot*               │
  └───────────────┬─────────────────────────────┘
                  │ selected text
                  ▼
  ┌──────────────────────────┐   config.json (hot-reloaded)
  │ llm.rs                   │◀── api_key · base_url · model
  │ POST /v1/messages        │    system_prompt
  │ stream: true (SSE)       │
  └───────────────┬──────────┘
                  │ text deltas (Tauri events)
                  ▼
  ┌──────────────────────────┐
  │ panel (NSPanel via       │
  │ tauri-nspanel), webview  │
  │ renders streaming        │
  │ markdown; Esc closes     │
  └──────────────────────────┘

  * restore is skipped if a third party changed the
    pasteboard while we held it (clipboard-manager truce)
```

Hotkey fires → text is captured through the pasteboard → streamed to Anthropic → deltas render live in a non-activating translucent panel near the mouse. Focus never leaves the app the user is typing in.

---

## 4. Epics

### Epic A — Core pipeline, ugly on purpose · MVP

**Goal:** hotkey → captured selection → streamed critique in an unstyled window, end to end.
**Success metrics:** capture works in the 5 daily-driver apps; first token < 1.5 s (p50); prompt validated on real texts (gate A3).

**Build order: A3 → A1 → A2.** Phase numbering is kept as-is for history; the order is not the
numbering. A3 is the gate with the kill criterion, and it becomes unexecutable the moment A1
and A2 exist — nobody kills a project after spending weekends on the plumbing. A3 depends on
neither. See decision #14.

#### Phase A3 — Prompt validation gate · **runs first**

Nothing here depends on A1 or A2: corpus evaluation is offline text-in, text-out, driven by
`evals/run.sh` (bash + curl, no Rust toolchain). Decision #14.

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| A3.1 | Draft system prompt v1 (L1-aware critique + variants + tags) | ✅ | `prompts/critique.md` |
| A3.2 | Eval harness: prompt variant × corpus → rating sheet | ✅ | `evals/run.sh` |
| A3.3 | Corpus: 20 real recent texts by Roman | ✅ | 20 texts, verbatim, gitignored |
| A3.4 | Iterate prompt against corpus; record verdicts | 🔄 | v1 + v2 run (2 of 3); **needs Roman's ratings** |

**Steps (detail):**

- **A3.1 — Prompt.** Deliverable: `prompts/critique.md` (copied into config default). Contract: author's L1 is Russian — hunt calques (articles, prepositions, "possibility to", tense aspect), tone register; output = short verdict → per-fragment critique with natural rewrites → closing ```json block: `{"tags": ["article-missing", ...]}`. The prompt carries two constraints the app cannot enforce for it: **never emit a whole-text rewrite** (decision #18) and **tags come from a fixed vocabulary** (decision #17).
- **A3.2 — Harness.** Deliverable: `evals/run.sh` — runs one prompt variant across the corpus at a given effort, machine-checks every tag block (parses? all tags in vocabulary? response truncated?), and writes `docs/corpus/results-<variant>-<effort>.md` with the rubric and blank rating columns. Resolves `api_key`/`base_url`/`model` from the app's own `config.json`, so it exercises the A2.1 config contract before `config.rs` exists. Outlives the gate as the regression suite for every later prompt edit.
- **A3.3 — Corpus.** Deliverable: `docs/corpus/` with 20 real texts (Slack messages, emails, post drafts). Roman supplies these; an agent must not invent them — synthetic texts contain the errors a model already knows how to find, so the gate would pass while telling you nothing. Mix and rationale in `docs/corpus/README.md`.
- **A3.4 — Iterate.** Deliverable: filled rating column in `docs/corpus/results-*.md`. Read the rubric in the generated sheet *before* the first output — one rater across three iterations will otherwise drift, and downward, exactly when the kill criterion needs a stable standard.

**Exit guardrails — Phase A3 → A1**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| Usefulness | ≥ 15/20 corpus outputs rated "useful" **or "correctly-silent"** by Roman | 🔲 | |
| Structure | ```json block parses **and every tag is in vocabulary** in 20/20 outputs | ✅ | **20/20 and 20/20**, `claude-sonnet-5` @ effort `medium`, 2026-08-26. Caveat: 2/20 outputs flagged a fragment without emitting a tag for it, so the issue never reaches the Epic E counts — surfaced as `⚠️ untagged` in the results table, to fix in the next prompt iteration |
| Misteaching | ≤ 2 outputs contain a "wrong item" (diagnostic; recorded per output) | 🔲 | |
| **Kill criterion** | If < 10/20 after 3 prompt iterations: stop, rethink the product before building any UI | 🔲 | |

`correctly-silent` was added after the corpus landed: roughly a quarter of real messages
("Feeling sick, taking off today") have nothing wrong with them, and a prompt that stays quiet
on those is behaving correctly — but under a useful/water/wrong rubric that correct behaviour
scores as a miss, so a perfect prompt could fail the gate. Decision #22.

---

#### Phase A1 — Scaffold, hotkey, capture

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| A1.1 | Tauri v2 scaffold; main window `"visible": false` in config | ✅ | accessory activation policy + tray icon |
| A1.2 | Global hotkey registration + menu-bar tray icon (quit, open config) | ✅ | verified by hand: 11 hotkey fires, both menu items |
| A1.3 | `capture.rs`: snapshot → ⌘C → poll → read → restore | ✅ | 7 unit tests; verified by hand incl. clipboard restore |
| A1.4 | Manual capture test checklist executed | ✅ | `docs/manual-tests.md` filled in — all green |

**Steps (detail):**

- **A1.1 — Scaffold.** ✅ Deliverable met: `npm run tauri dev` boots an app with a hidden window and a menu-bar tray icon. Window is created hidden so it can later be converted to a panel *before* first show. Also set here: `ActivationPolicy::Accessory`, so there is no Dock icon, no app menu, and the process never becomes the active application — the premise Epic B rests on. Frontend is Vite + vanilla TS (HMR for the C1 card iteration, no framework weight in a HUD). Tray art is the stock Tauri icon; a menu-bar template image is a C1 job.
- **A1.2 — Hotkey + tray.** ✅ Deliverable met: `⌥⌘E` (from config, `"Alt+Cmd+E"`) fires a Rust handler that logs; tray menu has Quit and Open Config, which opens the JSON in the default editor. Brings in a minimal `config.rs` — `load()` and `ensure_exists()` only, reading the exact file `evals/run.sh` already writes. **No watcher and no hot reload: that is still A2.1.** `ensure_exists()` is borrowed from A2.1 because Open Config needs a file to open on a clean machine; it writes mode 600.
- **A1.3 — Capture module.** ✅ Deliverable met: `capture::selection() -> Result<String, CaptureError>` plus 7 unit tests against a mocked pasteboard, one per battle-scar. The algorithm lives in `capture_with()` with the clipboard, keystroke and all three timings injected, so the scars are testable without a real pasteboard or a 2s wait. Capture runs on its own thread — blocking the hotkey handler for a 2s secure-input timeout would stall the UI and swallow the next press. Release builds log a character count only, never the user's text.
  ```rust
  pub fn selection() -> Result<String, CaptureError> {
      let snapshot = Pasteboard::snapshot();          // all types, not just text
      let before = Pasteboard::change_count();
      Enigo::new().key_sequence_cmd('c');             // CGEvent under the hood
      sleep(Duration::from_millis(50));               // min settle delay
      let text = poll_change(before, Duration::from_millis(5), Duration::from_secs(2))?;
      if Pasteboard::change_count() == before + 1 {   // nobody else touched it
          snapshot.restore();
      }                                               // else: leave it alone
      Ok(text)
  }
  ```
  Battle-scars to preserve (learned from WritingTools, reimplemented): the 50 ms settle delay, changeCount comparison instead of content comparison, and *skipping* restore when an external app (clipboard manager) altered the board mid-flight.
- **A1.4 — Manual checklist.** Deliverable: `docs/manual-tests.md` with a capture matrix, filled in. This code is untestable in CI; the checklist is the test artifact.

**Exit guardrails — Phase A1 → A2**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| Capture matrix | Selection captured in Slack, Chrome (GitHub textarea), Telegram, Mail.app, VS Code | ✅ | **5/5**, reported by Roman 2026-08-26 |
| Clipboard integrity | Prior clipboard contents (incl. an image) restored after capture; no fight with Raycast/Maccy clipboard history | ✅ | **pass** for text, image and RTF. An image seeds 9 pasteboard types; all are snapshotted and restored |
| Secure input | In a password field (secure input on): clean timeout error within 2 s, no hang, no crash | ✅ | **pass** — clean timeout, no hang. Capture runs off the hotkey thread, so the UI stays responsive |

#### Phase A2 — LLM pipeline into an unstyled window

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| A2.1 | Config loader with hot reload (fs watch) | ✅ | 8 unit tests; reload driven end-to-end |
| A2.2 | `llm.rs`: SSE client streaming deltas as Tauri events | ✅ | 12 parser tests; verified live end-to-end |
| A2.3 | Webview renders the markdown stream; Esc closes + aborts | ✅ | abort proven: no `critique done` after Esc |

**Steps (detail):**

- **A2.1 — Config.** ✅ Deliverable met. `ConfigStore` holds `Loaded { config, system_prompt }` behind an `Arc<RwLock<_>>`; `config::watch` reloads both files live. Two implementation notes worth keeping: it watches the **directories**, not the files, because editors save by writing a temp file and renaming it over the target — a watch on the file follows the old inode and goes silent after the first save. And change detection compares **content**, not events: one save fires several fs events, so the cheap re-read plus a diff is simpler than debouncing and cannot miss a change the way a time window can. Rebinding the hotkey live is the visible proof. A malformed config degrades to defaults and logs, rather than stopping a running app.
- **A2.2 — SSE client.** Deliverable: `llm::critique(text, config)` posting to `{base_url}/v1/messages` with `stream: true`, the prompt in the **`system` field** (never concatenated into the user message), emitting `content_block_delta` texts as `critique-delta` events; request holds an `AbortHandle`.
- **A2.3 — Render.** Deliverable: plain window (not yet a panel) that opens on hotkey, streams markdown, closes on Esc, and aborts the in-flight request on close — no token burn after dismissal.

**Exit guardrails — Phase A2 → Epic B**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| Latency | First visible token < 1.5 s p50 over 10 runs | 🔄 | **n=8: 1199, 1215, 1336, 1357, 1369, 1452, 1614, 4058 ms** (sonnet-5, effort medium). Median **1363 ms**, under the bar; 2/8 exceeded 1500 ms and the 4058 ms outlier was a first call after a rebuild (cold connection). Two more runs completes the sample. Effort is the lever if it fails (decision #16) |
| Abort | Esc mid-stream → request cancelled (verified in logs), window closes | ✅ | 2026-08-26: `first token in 1215 ms` → `dismissed — request aborted`, **with no `critique done` line following** — the stream really was killed, not just hidden |
| Config loop | Edit system_prompt in editor → next invocation uses it, no restart | ✅ | Reload verified 2026-08-26 (prompt edit → `prompt reloaded`, no restart; identical content correctly produced **no** reload; hotkey rebound live; malformed JSON logged and the app stayed up). `llm.rs` reads `system_prompt` from the store per invocation, so the next press uses the edited prompt |

---

### Epic B — The panel · MVP

**Goal:** the unstyled window becomes a non-activating floating NSPanel that never steals focus from the app being typed in.
**Success metrics:** user can keep typing in the source app while the panel is visible; zero focus flicker.

#### Phase B1 — NSPanel conversion

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| B1.1 | Pin `tauri-nspanel` to a commit; convert window pre-show | ✅ | pinned to `c9ec213`; converted while hidden |
| B1.2 | Disable both focus paths; floating level; all-spaces behavior | ✅ | verified: focus stays in Slack |
| B1.3 | Position at mouse; clamp to screen; multi-monitor coords | ✅ | all maths stays in AppKit coords — decision #29 |
| B1.4 | Dismissal: Esc, click-outside, auto-hide on app switch | ✅ | global NSEvent monitors — decision #30 |

**Steps (detail):**

- **B1.1 — Conversion.** Deliverable: hidden window converted to NSPanel at startup, before first show. Conversion after show = one frame of focus theft.
- **B1.2 — Focus discipline.** Deliverable: panel with `canBecomeKeyWindow = false` **and** `NSWindowStyleMaskNonactivatingPanel`. These kill two *different* activation paths; both are required. Level `.floating`, visible over full-screen apps (collection behavior).
- **B1.3 — Positioning.** Deliverable: panel appears at the mouse cursor, clamped to the current screen's visible frame. AX top-left vs AppKit bottom-left coordinate conversion handled per-screen (classic two-monitor "panel off-screen" bug).
- **B1.4 — Dismissal.** Deliverable: since the panel never becomes key, Esc lands in the *source app* — so dismissal uses a local+global event monitor for Esc, click-outside detection, and hide on frontmost-app change. This is the subtle step of the epic; budget time for it.

**Exit guardrails — Phase B1 → Epic C**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| No focus theft | Invoke while typing in Slack: caret stays, typing continues uninterrupted, menu bar still shows Slack | ✅ | **pass**, reported by Roman 2026-08-26 — "focus stays in slack" |
| Dismissal | Esc / click-outside / ⌘Tab all hide the panel; stream aborted | ✅ | **pass** — 4 dismissals logged, every one `dismissed — request aborted`, including one mid-stream (first token 1336 ms, no `critique done` after) |
| Multi-monitor | Correct position on a 2-display setup with different resolutions | 🔄 | Covered by Roman's "all works", but **not separately confirmed** and vacuous if only one display was attached. Re-check on a real 2-display setup before trusting it |

---

### Epic C — Polish · MVP

**Goal:** the panel is something you *want* to summon: translucent, typographically clean, instant-feeling.
**Success metrics:** hotkey → panel visible < 300 ms; Roman uses it daily for a full week without irritation.

#### Phase C1 — Look and feel

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| C1.1 | Vibrancy/translucency (window-vibrancy crate), card design | 🔲 | design call — needs human |
| C1.2 | Streaming render polish: highlight fragments, variants layout | 🔲 | |
| C1.3 | Panel pre-warm: keep created+hidden, show instantly | 🔲 | |
| C1.4 | Accessibility-permission onboarding flow | 🔲 | |

**Steps (detail):**

- **C1.1 — Card.** Deliverable: HUD-style translucent card; original fragments visually paired with natural rewrites. Roman approves the design direction before implementation detail.
- **C1.2 — Render.** Deliverable: the trailing ```json block is detected and stripped from the rendered stream (parsed silently for Epic E); user only ever sees prose.
- **C1.3 — Pre-warm.** Deliverable: webview stays alive hidden; show is repositioning + unhide, no cold WKWebView start on the hot path.
- **C1.4 — Onboarding.** Deliverable: first-run screen prompting Accessibility permission (`AXIsProcessTrustedWithOptions`) with a note that macOS requires re-granting after app updates.

**Exit guardrails — Phase C1 → MVP done**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| Speed | Hotkey → visible panel < 300 ms | 🔲 | |
| Dogfood week | 7 consecutive days of real use; Roman's verdict recorded here | 🔲 | |

---

### Epic D — Caret positioning · Post-MVP (v1.1)

**Goal:** panel appears at the text caret, IDE-inline-hint style, instead of at the mouse.
**Success metric:** caret positioning works in native apps; degrades gracefully elsewhere.

Rough plan: Swift shim `get_caret_rect()` exposed via `@_cdecl`, built with `swift-rs` (AX is a C API; raw Rust FFI rejected — decision #9). Three-tier fallback: bounds of selection range → `kAXFrame` of the focused element → mouse position. `AXUIElementSetMessagingTimeout` ≈ 200 ms so a busy target app can't hang the hotkey. Known risk: AX bounds are unreliable in Chromium/Electron — which is precisely why this is not MVP.

---

### Epic E — Error journal · Post-MVP (v2)

**Goal:** the app's long-term moat — accumulate the ```json error tags per critique and surface recurring weaknesses ("14 article misses before abstract nouns this month").
**Success metric:** weekly digest that changes what Roman practices.

Rough plan: append tags + timestamp to a local store (SQLite via `rusqlite`, or JSONL if that's overkill); a "weekly digest" entry point (tray menu) that aggregates counts, trends, and asks the LLM for one targeted exercise. No cloud, no accounts — local only. Design deliberately deferred; the only v1 obligation is the response contract already in place (decision #6).

---

## 5. Risk register

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Prompt yields generic advice, not real critique | Med | **Fatal** | Phase A3 is a hard gate with a kill criterion — before any UI investment |
| `tauri-nspanel` breakage/abandonment (git dependency) | Med | High | Pin to commit; the panel code is isolated in one module; worst case is a Swift shim for the panel too |
| Focus stealing regressions (the two-path problem) | Med | High | Guardrail B1 is explicit about it; manual test on every Tauri/plugin bump |
| Clipboard managers racing the snapshot/restore | Med | Med | changeCount discipline from A1.3; tested against Raycast clipboard history in guardrail A1 |
| Secure-input fields blocking synthetic ⌘C | High | Low | Graceful 2 s timeout with a human-readable error; documented limitation |
| Accessibility permission reset after each dev rebuild | High | Low (annoyance) | Documented in README dev section; stable signing identity reduces it |
| Apple ships a "teach me" / learner mode in Writing Tools | Low | High | Personal tool, not a business — impact is motivational, not financial. Journal + own-prompt control remain out of Apple's scope; re-evaluate at each WWDC |

---

## 6. Decision log

| # | Date | Decision | Context | Decided by |
|---|------|----------|---------|------------|
| 1 | 2026-08-26 | The app never modifies the user's text | Core thesis: replacement tools fix the text and stagnate the skill; critique-only builds the skill | Roman |
| 2 | 2026-08-26 | Prototype-in-Raycast path rejected | Considered and declined twice; going straight to a native app | Roman |
| 3 | 2026-08-26 | Tauri over Swift/SwiftUI | UI is the differentiator; CSS iteration speed wins; native surface small | Roman |
| 4 | 2026-08-26 | Clipboard-based capture, not AX | AX capture fails in Electron/browsers — exactly the target apps; ⌘C simulation is universal (technique observed in WritingTools) | Roman |
| 5 | 2026-08-26 | Fork of WritingTools rejected; zero code copied (GPL-3.0) | Its architecture is built around replace-text commands; only ~2 files were relevant; techniques reimplemented instead | Roman |
| 6 | 2026-08-26 | Response contract includes trailing json tags from day 1 | Journal ships in v2, but retraining a prompt/parser later is costlier than carrying tags now | Roman |
| 7 | 2026-08-26 | Anthropic-only + configurable base_url in v1 | Multi-provider cut from MVP; base_url covers proxies/compatible APIs | Roman |
| 8 | 2026-08-26 | Config = JSON file, no settings UI in MVP | Prompt tuning happens in an editor many times a day | Roman |
| 9 | 2026-08-26 | Caret positioning via Swift shim (swift-rs), deferred to v1.1 | AX is a C API; ~120 lines of unsafe Rust FFI vs 15 lines of Swift; mouse position is good enough for MVP | Roman |
| 10 | 2026-08-26 | Ugly-first build order: pipeline+prompt before any styling | The risky unknown is critique quality, not the panel | Roman |
| 11 | 2026-08-26 | Repo visibility: undecided | No commercial plans stated; all deps MIT/Apache so either way stays open | Roman |
| 12 | 2026-08-26 | Pace: evenings/weekends | Side project next to full-time work | Roman |
| 13 | 2026-08-26 | Proceed despite overlap with Apple Intelligence Writing Tools | Apple (macOS 26) covers select→popup→proofread-with-explanations, incl. custom rewrite prompts. Overlap acknowledged; redpen's value is concentrated in never-replace flow, L1-aware naturalness critique, and the error journal — see §1 Positioning. Panel UX itself is commodity | Roman |
| 14 | 2026-08-26 | Phase A3 runs before A1 and A2 | The kill criterion says "stop before building any UI", but it is unexecutable once A1+A2 exist — sunk cost decides instead. A3 depends on neither; the harness is bash + curl. Reaching the gate costs one evening rather than several weekends | Roman |
| 15 | 2026-08-26 | ~~Default model `claude-opus-5`~~ **— superseded by #25** | Closes the open question. Critique quality *is* the product, so no downgrade for cost; at personal volume on short texts the difference is rounding error. `model` stays config-driven, so it is a one-line change | Roman |
| 16 | 2026-08-26 | Latency is tuned with `effort`, never by disabling thinking | On Opus 5 adaptive thinking is on by default and thinking `display` defaults to `"omitted"` — the panel would show a blank card for seconds and fail the A2 guardrail as written. `thinking: {type: "disabled"}` is rejected as the fix: it leaks `<thinking>` tags and can write tool calls into visible text. Use `effort: low`/`medium`. Consequence: A3 must evaluate at the shipping effort, or corpus results do not transfer | Roman |
| 17 | 2026-08-26 | Tag vocabulary fixed at 20 tags for v1 | Freeform tags make Epic E unaggregatable — "14 article misses this month" needs a stable key. Tags repeat per occurrence rather than dedupe, because frequency is the whole signal. The list lives in the prompt; the harness extracts and validates against it | Roman |
| 18 | 2026-08-26 | The prompt must never emit a whole-text rewrite | Decision #1 is enforced in the *app* by having no paste path — but the prompt can defeat it unaided: a clean corrected version in the output just gets copied, and the pedagogy is gone. Per-fragment rewrites only | Roman |
| 19 | 2026-08-26 | Eval harness is bash + curl, and is kept permanently | Reaches the gate with no Rust toolchain, then becomes the prompt regression suite after v1. Rust has no official Anthropic SDK, so raw HTTP is the path in the app too | Roman |
| 20 | 2026-08-26 | Frontend is Vite + vanilla TypeScript, no framework | Decision #3 bought Tauri for CSS iteration speed; a framework adds weight without helping a single streaming card. Vite gives HMR, which is the part that actually matters for C1 | Roman |
| 21 | 2026-08-26 | macOS activation policy is `Accessory`, set at scaffold time | Not cosmetic: a regular-policy app activates when its window shows, which would defeat Epic B before it starts. Cheaper to set now than to debug as focus theft later | Roman |
| 22 | 2026-08-26 | Rubric gains a `correctly-silent` pass label | The corpus turned out to contain several short messages with genuinely nothing to critique. Under useful/water/wrong, correct restraint on those scores as a miss, so a prompt behaving exactly as designed could trip the kill criterion. Silence on a clean text is a pass, not a failure — the bar stays at 15/20 | Roman |
| 23 | 2026-08-26 | Config references the prompt by path; the live JSON carries no comments | Two corrections to A2.1 as originally written. Inlining a 141-line prompt as an escaped JSON string is unreadable and uneditable, which defeats decision #8's own rationale — so it is `system_prompt_path`, pointing at the repo so no second copy can drift. And a "commented default" is not buildable: JSON has no comments and both `jq` and `serde_json` reject them, verified. The annotated copy is `config.example.jsonc`, which nothing parses | Roman |
| 24 | 2026-08-26 | `fallbacks` is gated on the model tier, not sent unconditionally | Verified against the live API: `'claude-sonnet-5' does not support the fallbacks parameter` — it is Opus-5/Fable-5-tier only. The harness now sends the parameter and its beta header only for those models, so switching `model` in config cannot silently 400 every call | Roman |
| 25 | 2026-08-26 | Default model is `claude-sonnet-5` — reverses #15 | Roman's call after run 1 of the gate. $2/$10 per MTok against Opus 5's $5/$25, on a tool fired dozens of times a day. Two consequences carried forward: this tier rejects `fallbacks` (#24), and the A3 gate now certifies *this* model plus the prompt — switching back to Opus means re-running the corpus, for the same reason effort must match production (#16) | Roman |
| 26 | 2026-08-26 | Prompt v2 adds a `typo` tag, quarantined from the Epic E digest | Without a bucket for them, typos were being misfiled into *learnable* categories — run 1 tagged the doubled article in "the the build-up" as `article-extra`, which would have shown up in the journal as an article weakness it is not. Tagged for integrity, excluded from the digest: typo frequency is not something to practise. **Measured cost:** the tag reads to the model as licence — it fired in 5/20 outputs and `15-text` lost its correctly-silent verdict to a note about capitalising "i". The "never the only thing you say" cap is advisory and did not hold; v3 must make it mechanical | Roman |
| 27 | 2026-08-26 | Synthetic shortcuts use raw keycodes, never `Key::Unicode` | `Key::Unicode('c')` resolves the character through the *active keyboard layout*; when that lookup fails — which it does with a non-Latin layout frontmost, and this app is built for a Russian speaker — enigo falls back to keycode 0. `kVK_ANSI_A` **is** 0, so redpen sent ⌘A: the target app selected all, nothing reached the pasteboard, and capture failed looking like secure input. Now `kVK_ANSI_C` (0x08) directly. Note the 7 capture unit tests could not have caught this: they inject the keystroke as a closure, so the one wrong line is never exercised — only the manual matrix covers it | Roman |
| 28 | 2026-08-26 | All AppKit calls are dispatched to the main thread inside `panel.rs` | Cost a hard crash (`SIGTRAP`, `"Must only be used from the main thread"`, `-[NSWindow _doOrderWindow:]`): capture deliberately runs on its own thread, and it called `order_front_regardless` straight into AppKit. Tauri's own `window.show()` hides this by dispatching internally; `tauri-nspanel` sends the ObjC message directly, so swapping one for the other silently moved a thread requirement onto the caller. The dispatch now lives behind `panel::show`/`panel::hide` so no call site can reintroduce it | Roman |
| 29 | 2026-08-26 | Panel positioning never leaves AppKit screen coordinates | `mouseLocation`, `NSScreen::frame`, `visibleFrame` and `setFrameOrigin` all share one bottom-left, y-up space spanning every display, so containment and clamping are plain comparisons. The "classic two-monitor off-screen bug" the spec warns about comes from mixing that with the top-left y-down space Tauri's `set_position` and the AX APIs use — the flip needs the *primary* screen's height, and taking it from the wrong screen throws the panel a whole display away. Never converting cannot convert wrongly | Roman |
| 30 | 2026-08-26 | Dismissal uses global NSEvent *monitors*, not a registered Esc shortcut | A non-activating panel never becomes key, so no keystroke reaches the webview and the A2.3 JS listener is permanently dead code (removed, not patched). Registering Esc as a global shortcut was the alternative and was rejected: it would swallow Esc system-wide while the panel is open, breaking vim, dialogs and every modal. A global monitor observes without consuming, so Esc both reaches the source app and dismisses the panel | Roman |

---

## 7. Open questions

- [ ] Final app name — `redpen` is a working title
- [ ] Repo public or private (decision #11 pending)
- [x] ~~Default model string for the config template~~ → `claude-sonnet-5`, decision #25 (reverses #15)
- [ ] Distribute signed+notarized builds, or personal-use-only from source?
- [ ] Default hotkey — `⌥⌘E` assumed, unverified against Roman's existing bindings
- [ ] API key sits in plaintext in `config.json` — accept for v1, or move to Keychain? Interacts with repo visibility and with the hot-reload-in-an-editor workflow (decision #8)

---

## How to Update This Document

This spec is the source of truth for the build. Keep it current as work happens:

- **Status markers.** Update a step's status in its tracker table as you go: 🔲 → 🔄 → ✅. Use ⏸️ for blocked (note why in Notes) and ❌ for cut (leave the row; the strikethrough of history is useful).
- **Current focus.** Keep the pointer at the top aimed at the next actionable 🔲 step. Update it the moment you finish a step or cross a phase boundary — a stale pointer is worse than none, since it sends the next reader to the wrong place.
- **Guardrails.** When you hit a phase boundary, fill the **Actual outcome** column with what really happened and set the guardrail status. Don't advance to the next phase until its entry guardrails pass — or log a decision explaining why you're proceeding anyway.
- **Decisions.** Any non-trivial choice made during the build gets a new row in the Decision Log (§6). It's append-only — reversals are new rows, not edits. If the choice changes the architecture, also update the Technical Decisions snapshot (§2).
- **Spec changes.** Structural changes (new epic, re-scoped phase) get a Changelog row at the top. Keep the executive summary honest if the project's shape shifts.
- **Open questions.** When one resolves, strike it from §7 and log the decision in §6.
