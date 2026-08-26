# redpen

A macOS menu-bar app that critiques your English instead of correcting it. Select text
anywhere, hit the hotkey, read what sounds off and why — then retype it yourself. It never
inserts, replaces, or pastes anything; that is the whole point.

Full plan, phases and decisions: [SPEC.md](SPEC.md).

## Status

Pre-MVP. The build order is **A3 → A1 → A2** (decision #14): the prompt has to prove itself
against real writing before any of the app is worth building.

| | |
|---|---|
| A3.1 prompt | ✅ `prompts/critique.md` |
| A3.2 eval harness | ✅ `evals/run.sh` |
| A3.3 corpus | 🔲 **needs 20 real texts** — see `docs/corpus/README.md` |
| A3.4 iterate + rate | 🔲 blocked on the corpus |
| A1.1 scaffold | ✅ this repo boots |
| A1.2 hotkey + tray | ✅ ⌥⌘E fires; tray has Quit / Open Config |
| A1.3 capture | ✅ 7 unit tests + verified in 5 apps |
| A1.4 test matrix | ✅ `docs/manual-tests.md`, all green |
| A2.1 config + hot reload | ✅ watches config.json and the prompt file |

## Layout

```
prompts/critique.md    system prompt — the actual product
evals/run.sh           prompt × corpus → rating sheet (bash + curl, no Rust)
docs/corpus/           20 real texts + results. Gitignored: real writing stays local
src-tauri/             the Rust app
src/, index.html       the webview
```

## Running the prompt gate

No Rust toolchain needed. The first run writes
`~/Library/Application Support/redpen/config.json` (mode 600) and tells you where it is; put
your key in its `api_key` field, or export `ANTHROPIC_API_KEY` for a one-off. The harness and
the app read that same file, so a key or a model is set in exactly one place.

`config.example.jsonc` documents every field. It is **documentation only** — the live config
must be strict JSON, because both `jq` and `serde_json` reject comments.

```sh
evals/run.sh -n      # what it sees, spends nothing
evals/run.sh         # run the prompt across the corpus
evals/run.sh -e medium -p prompts/v2.md    # sweep effort, try a variant
```

## Running the app

```sh
npm install
npm run tauri dev
```

The window is created **hidden** (`visible: false` in `tauri.conf.json`) so it can be
converted to an NSPanel before its first show — converting after the first show costs a frame
of focus theft (step B1.1). The app runs under the macOS *accessory* activation policy: no
Dock icon, no app menu, and it never becomes the active application. Look for the tray icon in
the menu bar; there is no window to see yet.

### Dev annoyance to expect

macOS ties Accessibility permission to the signed binary, so **every rebuild can reset it**
and the simulated ⌘C in `capture.rs` will silently stop working. When capture mysteriously
returns nothing, re-grant under System Settings → Privacy & Security → Accessibility before
debugging anything else. A stable signing identity reduces how often this bites.
