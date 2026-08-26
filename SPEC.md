# redpen — Technical Spec

**Author:** Roman Pronskiy · **Created:** 2026-08-26

> 📄 **This is a living document.** Status markers, decisions, and guardrail outcomes are meant to be updated as the work happens. See [How to Update This Document](#how-to-update-this-document) before editing.

### Changelog

| Date | Change | Author |
|------|--------|--------|
| 2026-08-26 | Initial spec created | Roman |
| 2026-08-26 | Added §1 Positioning vs Apple Intelligence; decision #13; sherlocking risk | Roman |

### Status legend

🔲 Not started · 🔄 In progress · ✅ Done · ⏸️ Blocked · ❌ Cut

### Current focus

**Now on:** Epic A → Phase A1 → step A1.1 — scaffold the Tauri project with a hidden main window.

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
| Config | JSON at `~/Library/Application Support/redpen/config.json`, hot-reloaded | The system prompt gets edited dozens of times a day during tuning; an editor beats any settings UI |
| Response contract | Markdown critique + trailing ```json block with error tags | Journal (Epic E) needs structured tags from day one even though the journal ships later. Tags at the *end* so the user reads prose, not streaming JSON |
| Core principle | **Never insert, replace, or paste anything** | The entire thesis. No paste-back code path exists in this codebase |
| Caret positioning | Deferred to Epic D via a Swift shim (`swift-rs`) | Raw AX C API from Rust is ~120 lines of unsafe FFI; a `@_cdecl` Swift function is 15 lines. Panel positions at the mouse cursor until then |
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

#### Phase A1 — Scaffold, hotkey, capture

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| A1.1 | Tauri v2 scaffold; main window `"visible": false` in config | 🔲 | |
| A1.2 | Global hotkey registration + menu-bar tray icon (quit, open config) | 🔲 | |
| A1.3 | `capture.rs`: snapshot → ⌘C → poll → read → restore | 🔲 | |
| A1.4 | Manual capture test checklist executed | 🔲 | |

**Steps (detail):**

- **A1.1 — Scaffold.** Deliverable: `npm run tauri dev` boots an app with a hidden window and a tray icon. Window is created hidden so it can later be converted to a panel *before* first show.
- **A1.2 — Hotkey + tray.** Deliverable: hotkey (default `⌥⌘E`, configurable) fires a Rust handler that logs; tray menu has Quit and Open Config (opens the JSON in the default editor).
- **A1.3 — Capture module.** Deliverable: `capture::selection() -> Result<String, CaptureError>` plus unit tests for the snapshot/restore logic (pasteboard mocked).
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
| Capture matrix | Selection captured in Slack, Chrome (GitHub textarea), Telegram, Mail.app, VS Code | 🔲 | |
| Clipboard integrity | Prior clipboard contents (incl. an image) restored after capture; no fight with Raycast/Maccy clipboard history | 🔲 | |
| Secure input | In a password field (secure input on): clean timeout error within 2 s, no hang, no crash | 🔲 | |

#### Phase A2 — LLM pipeline into an unstyled window

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| A2.1 | Config loader with hot reload (fs watch) | 🔲 | |
| A2.2 | `llm.rs`: SSE client streaming deltas as Tauri events | 🔲 | |
| A2.3 | Webview renders the markdown stream; Esc closes + aborts | 🔲 | |

**Steps (detail):**

- **A2.1 — Config.** Deliverable: `config.rs` reading `{ api_key, base_url, model, system_prompt, hotkey }` from Application Support, creating a commented default on first run, reloading on file change. Unit-tested.
- **A2.2 — SSE client.** Deliverable: `llm::critique(text, config)` posting to `{base_url}/v1/messages` with `stream: true`, the prompt in the **`system` field** (never concatenated into the user message), emitting `content_block_delta` texts as `critique-delta` events; request holds an `AbortHandle`.
- **A2.3 — Render.** Deliverable: plain window (not yet a panel) that opens on hotkey, streams markdown, closes on Esc, and aborts the in-flight request on close — no token burn after dismissal.

**Exit guardrails — Phase A2 → A3**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| Latency | First visible token < 1.5 s p50 over 10 runs | 🔲 | |
| Abort | Esc mid-stream → request cancelled (verified in logs), window closes | 🔲 | |
| Config loop | Edit system_prompt in editor → next invocation uses it, no restart | 🔲 | |

#### Phase A3 — Prompt validation gate

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| A3.1 | Draft system prompt v1 (L1-aware critique + variants + tags) | 🔲 | |
| A3.2 | Corpus: 20 real recent texts by Roman | 🔲 | needs human |
| A3.3 | Iterate prompt against corpus; record verdicts | 🔲 | needs human |

**Steps (detail):**

- **A3.1 — Prompt.** Deliverable: `prompts/critique.md` (copied into config default). Contract: author's L1 is Russian — hunt calques (articles, prepositions, "possibility to", tense aspect), tone register; output = short verdict → per-fragment critique with natural rewrites → closing ```json block: `{"tags": ["article-abstract-noun", ...]}`.
- **A3.2 — Corpus.** Deliverable: `docs/corpus/` with 20 real texts (Slack messages, emails, post drafts). Roman supplies these; an agent must not invent them.
- **A3.3 — Iterate.** Deliverable: verdict table in `docs/corpus/results.md` — per text: useful / water / wrong.

**Exit guardrails — Phase A3 → Epic B**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| Usefulness | ≥ 15/20 corpus outputs rated "useful" by Roman | 🔲 | |
| Structure | ```json block parses in 20/20 outputs | 🔲 | |
| **Kill criterion** | If < 10/20 after 3 prompt iterations: stop, rethink the product before building any UI | 🔲 | |

---

### Epic B — The panel · MVP

**Goal:** the unstyled window becomes a non-activating floating NSPanel that never steals focus from the app being typed in.
**Success metrics:** user can keep typing in the source app while the panel is visible; zero focus flicker.

#### Phase B1 — NSPanel conversion

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| B1.1 | Pin `tauri-nspanel` to a commit; convert window pre-show | 🔲 | |
| B1.2 | Disable both focus paths; floating level; all-spaces behavior | 🔲 | |
| B1.3 | Position at mouse; clamp to screen; multi-monitor coords | 🔲 | |
| B1.4 | Dismissal: Esc, click-outside, auto-hide on app switch | 🔲 | |

**Steps (detail):**

- **B1.1 — Conversion.** Deliverable: hidden window converted to NSPanel at startup, before first show. Conversion after show = one frame of focus theft.
- **B1.2 — Focus discipline.** Deliverable: panel with `canBecomeKeyWindow = false` **and** `NSWindowStyleMaskNonactivatingPanel`. These kill two *different* activation paths; both are required. Level `.floating`, visible over full-screen apps (collection behavior).
- **B1.3 — Positioning.** Deliverable: panel appears at the mouse cursor, clamped to the current screen's visible frame. AX top-left vs AppKit bottom-left coordinate conversion handled per-screen (classic two-monitor "panel off-screen" bug).
- **B1.4 — Dismissal.** Deliverable: since the panel never becomes key, Esc lands in the *source app* — so dismissal uses a local+global event monitor for Esc, click-outside detection, and hide on frontmost-app change. This is the subtle step of the epic; budget time for it.

**Exit guardrails — Phase B1 → Epic C**

| Guardrail | Criteria (pass/fail) | Status | Actual outcome |
|-----------|----------------------|--------|----------------|
| No focus theft | Invoke while typing in Slack: caret stays, typing continues uninterrupted, menu bar still shows Slack | 🔲 | |
| Dismissal | Esc / click-outside / ⌘Tab all hide the panel; stream aborted | 🔲 | |
| Multi-monitor | Correct position on a 2-display setup with different resolutions | 🔲 | |

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

---

## 7. Open questions

- [ ] Final app name — `redpen` is a working title
- [ ] Repo public or private (decision #11 pending)
- [ ] Default model string for the config template
- [ ] Distribute signed+notarized builds, or personal-use-only from source?
- [ ] Default hotkey — `⌥⌘E` assumed, unverified against Roman's existing bindings

---

## How to Update This Document

This spec is the source of truth for the build. Keep it current as work happens:

- **Status markers.** Update a step's status in its tracker table as you go: 🔲 → 🔄 → ✅. Use ⏸️ for blocked (note why in Notes) and ❌ for cut (leave the row; the strikethrough of history is useful).
- **Current focus.** Keep the pointer at the top aimed at the next actionable 🔲 step. Update it the moment you finish a step or cross a phase boundary — a stale pointer is worse than none, since it sends the next reader to the wrong place.
- **Guardrails.** When you hit a phase boundary, fill the **Actual outcome** column with what really happened and set the guardrail status. Don't advance to the next phase until its entry guardrails pass — or log a decision explaining why you're proceeding anyway.
- **Decisions.** Any non-trivial choice made during the build gets a new row in the Decision Log (§6). It's append-only — reversals are new rows, not edits. If the choice changes the architecture, also update the Technical Decisions snapshot (§2).
- **Spec changes.** Structural changes (new epic, re-scoped phase) get a Changelog row at the top. Keep the executive summary honest if the project's shape shifts.
- **Open questions.** When one resolves, strike it from §7 and log the decision in §6.
