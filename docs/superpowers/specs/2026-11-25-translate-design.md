# Translate on double ⌘C — design

Short design for a small feature. Agreed with Roman on 2026-11-25.

## Why

When Roman hits text he doesn't understand, he copies it, opens a Google Translate tab, pastes,
reads, and switches back. This replaces that round trip: select, press ⌘C twice quickly, and the
translation shows up in Redpen's card next to the mouse, without leaving the app.

## What it does

- **Trigger:** two ⌘C presses within ~400 ms. A single ⌘C stays an ordinary copy and Redpen does
  nothing. The clipboard keeps what was copied; Redpen never restores it, because the copy was the
  user's own.
- **Direction is automatic, no setting:** English → Russian; any other language (Russian, Dutch,
  anything) → English.
- **Output (option B):** the translation, then short notes only where there is something worth
  explaining (an idiom, a false friend, a place where a literal reading misleads). Plain text gets
  no notes. *My call:* notes are written in Russian, Roman's first language, whatever the direction.
- **Same card as the critique:** non-activating, near the mouse, Esc or a click outside dismisses
  it and aborts the request.
- **Nothing selected:** if no copy lands after the second press, show nothing. A double ⌘C can be
  an accident, and an error card there would be noise.

## Goals / non-goals

- Goal: faster than the Google Translate tab, for any language pair above.
- Non-goal: replacing or pasting anything. Like the critique, it never writes into the user's text.
- Non-goal: a language picker or per-pair configuration.
- Anti-goal: breaking ordinary ⌘C anywhere, or making it feel laggy.

## Approaches

1. **Listen for the keystrokes (chosen).** A passive global `NSEvent` key-down monitor, the same
   mechanism `panel.rs` already uses for Esc. Watches for the physical C key (keycode 8) with
   Command, counts two within the window, then waits for the target app's copy to land on the
   pasteboard and reads it. Uses the Accessibility permission Redpen already has; never swallows ⌘C.
2. **Poll the pasteboard's change count.** No key listening, but a constant ~50 ms poll, and it
   fires on any copy (menu Copy, scripts, clipboard managers), so it can't tell a double ⌘C from
   noise. Rejected for false triggers.
3. **CGEventTap.** Same outcome as 1 with more code and possibly an extra Input Monitoring
   permission. Rejected.

## How (decided)

- **Detection:** keycode 8 + Command, no Shift/Option/Control, ignore auto-repeat. Two presses
  within ~400 ms trigger. Note the pasteboard change count at the first press; after the second,
  wait (up to ~300 ms) for it to move past that, then read the text. No change → silent.
- **Don't trigger on our own keystrokes:** the ⌥⌘E capture sends synthetic ⌘C, up to twice. The
  monitor must ignore presses while a capture is in progress.
- **LLM:** `llm::run` / `build_body` take the prompt, model, effort and whether to think as
  parameters instead of reading the critique's fixed fields. Translation and critique share the
  streaming, abort (`InFlight`) and error paths.
- **Prompt:** `prompts/translate.md`, bundled into the binary (`include_str!`), so translation works
  with zero setup. An optional `translate_prompt_path` overrides it and hot-reloads like the
  critique prompt.
- **Config (new, all optional, hot-reload):** `translate_model` (*my call:* default
  `claude-haiku-5-5`, the fast one; speed is the point), `translate_effort` (default `low`), and
  `translate_prompt_path`. Thinking is off for translation. `config.example.jsonc` and the README
  document them.
- **Card:** a translation-mode render in `src/main.ts`: the translation as the main block, then
  notes as a short list. Separate events (or a mode flag on `critique-start`) so the critique
  parser never sees translation output.
- **Card already open:** a double ⌘C replaces whatever it shows and aborts the request in flight.

## Left to the builder

- The exact output shape of the translate prompt and the parser that matches it.
- Event names, and whether translation gets its own events or a mode flag.
- Exact timings (400 ms / 300 ms) once tried by hand.
- Unit tests for the double-press detector (inject time, as `capture_with` injects its clock and
  keystroke), the direction rule's prompt examples, and the render parser. A manual-test entry in
  `docs/manual-tests.md`.
