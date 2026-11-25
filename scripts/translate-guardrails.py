#!/usr/bin/env python3
"""Epic F guardrails (SPEC F1), driven with System Events against the real apps.

    npm run tauri dev > /tmp/redpen-dev.log 2>&1 &
    scripts/translate-guardrails.py /tmp/redpen-dev.log         # F1: triggers, across apps
    scripts/translate-guardrails.py /tmp/redpen-dev.log --f2    # F2: speed, abort, replace

Needs Accessibility for whatever runs it, and Safari, Terminal, VS Code, Slack and TextEdit.
Never presses Return anywhere, so nothing pasted into Slack can be sent; the draft is cleared.

Terminal is expected to fail while Secure Keyboard Entry is on: macOS then hides every
keystroke from other apps, redpen's monitor included, and blocks this script's own keys too.
Worse, a running Terminal can keep secure input on from the background and blind the monitor
everywhere, so the script quits Terminal at the end if it was not running before.

Best effort. Focus is the weak spot: if ⌘A lands in Safari's address bar or an empty VS Code
editor, a check fails on the script's fault, not redpen's — the log line it prints says which
(a URL translated, or "no text was copied"). Rerun, or check that app by hand.
"""
import os, subprocess, sys, time

import tempfile
LOG = sys.argv[1]
SCRATCH = tempfile.mkdtemp(prefix="redpen-guardrails-")
HERE = os.path.dirname(os.path.abspath(__file__))
INPUTSOURCE = os.path.join(SCRATCH, "inputsource")
subprocess.run(["swiftc", "-O", os.path.join(HERE, "inputsource.swift"), "-o", INPUTSOURCE], check=True)
results = []


def osa(script):
    subprocess.run(["osascript", "-e", script], capture_output=True, text=True)


def key(code, mods=""):
    using = f" using {{{mods}}}" if mods else ""
    osa(f'tell application "System Events" to key code {code}{using}')


cmd_a = lambda: key(0, "command down")
cmd_c = lambda: key(8, "command down")
cmd_v = lambda: key(9, "command down")
esc = lambda: key(53)


def double_c():
    osa('tell application "System Events"\nkey code 8 using command down\ndelay 0.12\nkey code 8 using command down\nend tell')


def lines():
    with open(LOG, errors="replace") as f:
        return [l for l in f.read().splitlines() if "[redpen]" in l]


def since(n):
    return lines()[n:]


def wait_for(n, needle, timeout):
    end = time.time() + timeout
    while time.time() < end:
        if any(needle in l for l in since(n)):
            return True
        time.sleep(0.2)
    return False


def record(name, ok, detail=""):
    results.append((name, ok, detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}  {detail}", flush=True)


def activate(app):
    osa(f'tell application "{app}" to activate')
    time.sleep(1.0)


def pbpaste():
    return subprocess.run(["pbpaste"], capture_output=True, text=True).stdout


def check_double(label):
    n = len(lines())
    double_c()
    fired = wait_for(n, "double ⌘C:", 2)
    done = wait_for(n, "translation done", 8)
    esc()
    time.sleep(0.5)
    record(f"double ⌘C triggers · {label}", fired and done,
           "" if fired and done else " | ".join(since(n))[-200:])


def check_singles(label, count, expect):
    # A sentinel first, or a test that put the text on the clipboard itself proves nothing.
    subprocess.run(["pbcopy"], input="redpen-sentinel", text=True)
    n = len(lines())
    for _ in range(count):
        cmd_c()
        time.sleep(0.7)
    time.sleep(0.8)
    fired = [l for l in since(n) if "double ⌘C" in l]
    record(f"{count} single ⌘C, no translation · {label}", not fired, "; ".join(fired))
    got = pbpaste()
    record(f"⌘C copies · {label}", expect in got, "" if expect in got else repr(got[:80]))
    return got


def check_paste(label, expect):
    activate("TextEdit")
    osa('tell application "TextEdit" to set text of front document to ""')
    cmd_v()
    time.sleep(0.5)
    r = subprocess.run(["osascript", "-e", 'tell application "TextEdit" to get text of front document'],
                       capture_output=True, text=True).stdout
    record(f"⌘V pastes · {label}", expect in r, "" if expect in r else repr(r[:80]))


def f2():
    """Speed, abort and replace, all in one TextEdit document."""
    import re, statistics
    texts = [l.strip() for f in sorted(os.listdir(os.path.join(HERE, "..", "docs", "translate-fixtures")))
             if f.endswith(".txt") for l in [open(os.path.join(HERE, "..", "docs", "translate-fixtures", f)).read()]]
    osa('tell application "TextEdit" to activate\ntell application "TextEdit" to make new document with properties {text:""}')
    time.sleep(1)

    def put(text):
        osa('tell application "TextEdit" to set text of front document to ' + '"' + text.replace('"', '\\"') + '"')
        activate("TextEdit"); cmd_a()

    # Speed: press → first visible token, n=10.
    ms = []
    for t in texts[:10]:
        put(t)
        n = len(lines())
        double_c()
        wait_for(n, "translation done", 10)
        for l in since(n):
            m = re.search(r"\((\d+) ms since the press\)", l)
            if m: ms.append(int(m.group(1)))
        esc(); time.sleep(0.6)
    p50 = statistics.median(ms) if ms else None
    record("speed: press → first token p50 < 1000 ms", bool(ms) and len(ms) == 10 and p50 < 1000,
           f"n={len(ms)} p50={p50} ms min={min(ms, default=None)} max={max(ms, default=None)} all={ms}")

    # Abort: Esc while the translation is streaming.
    long = " ".join(["The quarterly review slipped because the vendor kept moving the goalposts, "
                     "and every time we thought we had nailed it down, legal came back with another round of edits."] * 6)
    put(long)
    n = len(lines())
    double_c()
    wait_for(n, "first token", 10)
    time.sleep(0.2); esc(); time.sleep(4)
    after = since(n)
    aborted = any("request aborted" in l for l in after)
    finished = any("translation done" in l for l in after)
    record("abort: Esc mid-stream stops the request", aborted and not finished, " | ".join(after)[-240:])

    # Replace: a double ⌘C while a critique streams.
    put("I have a possibility to join tomorrow, but I am not sure that I will be able to make a presentation.")
    n = len(lines())
    key(14, "option down, command down")
    wait_for(n, "first token", 12)
    activate("TextEdit"); cmd_a(); double_c()
    wait_for(n, "translation done", 12); time.sleep(1)
    after = since(n)
    crit_done = any("critique done" in l for l in after)
    trans_done = any("translation done" in l for l in after)
    record("replace: double ⌘C over a streaming critique", trans_done and not crit_done, " | ".join(after)[-240:])
    esc(); time.sleep(0.5)
    osa('tell application "TextEdit" to close front document saving no')
    print(f"\n{sum(ok for _, ok, _ in results)}/{len(results)} checks passed")


if "--f2" in sys.argv:
    f2()
    sys.exit(0)

terminal_was_running = subprocess.run(["pgrep", "-x", "Terminal"], capture_output=True).returncode == 0
original_layout = subprocess.run([INPUTSOURCE], capture_output=True, text=True).stdout.strip()
subprocess.run([INPUTSOURCE, "com.apple.keylayout.US"], capture_output=True)

# ---- fixtures -------------------------------------------------------------------------
html = os.path.join(SCRATCH, "f1-safari.html")
with open(html, "w") as f:
    f.write("<!doctype html><meta charset=utf-8><p>Het project loopt als een trein, maar de deadline blijft krap.</p>")

osa('tell application "TextEdit" to activate\ntell application "TextEdit" to make new document with properties {text:""}')
time.sleep(1)

# ---- TextEdit ---------------------------------------------------------------------------
te_text = "I can't carve out time this week, let's touch base on Monday."
osa(f'tell application "TextEdit" to set text of front document to "{te_text}"')
activate("TextEdit"); cmd_a()
check_double("TextEdit")
check_singles("TextEdit", 2, te_text)

# ---- Safari -----------------------------------------------------------------------------
subprocess.run(["open", "-a", "Safari", html]); time.sleep(3)
activate("Safari")
# Click into the page, or ⌘A selects the address bar and the test copies the URL.
b = subprocess.run(["osascript", "-e", 'tell application "Safari" to get bounds of front window'],
                   capture_output=True, text=True).stdout.strip().split(", ")
x1, y1, x2, y2 = map(int, b)
osa(f'tell application "System Events" to tell process "Safari" to click at {{{(x1 + x2) // 2}, {y1 + 250}}}')
time.sleep(0.5); cmd_a()
check_double("Safari")
check_singles("Safari", 12, "loopt als een trein")
check_paste("from Safari", "loopt als een trein")

# ---- VS Code ----------------------------------------------------------------------------
# A new untitled editor filled by paste, so focus is certainly in it.
vs_text = "Let's not reinvent the wheel here: we can piggyback on the existing auth flow."
subprocess.run(["pbcopy"], input=vs_text, text=True)
activate("Visual Studio Code"); time.sleep(1)
key(45, "command down"); time.sleep(1.5)   # ⌘N
cmd_v(); time.sleep(0.5); cmd_a()
check_double("VS Code")
check_singles("VS Code", 12, vs_text)
check_paste("from VS Code", vs_text)
activate("Visual Studio Code"); cmd_a(); key(51); time.sleep(0.3)

# ---- Slack: a draft only, never sent ----------------------------------------------------
slack_text = "Kun je even checken of de factuur al betaald is? Ik heb er een hard hoofd in."
subprocess.run(["pbcopy"], input=slack_text, text=True)
activate("Slack"); time.sleep(1)
cmd_v(); time.sleep(0.5); cmd_a()
check_double("Slack")
check_singles("Slack", 12, slack_text)
activate("Slack"); cmd_a(); key(51)  # clear the draft: select all, Delete. Never Return.
check_paste("from Slack", slack_text)

# ---- Russian layout ---------------------------------------------------------------------
subprocess.run([INPUTSOURCE, "com.apple.keylayout.Russian"], capture_output=True)
time.sleep(0.5)
osa(f'tell application "TextEdit" to set text of front document to "{te_text}"')
activate("TextEdit"); cmd_a()
check_double("TextEdit, Russian layout")
activate("Safari"); cmd_a()
check_double("Safari, Russian layout")
subprocess.run([INPUTSOURCE, "com.apple.keylayout.US"], capture_output=True)

# ---- ⌥⌘E is not a trigger --------------------------------------------------------------
osa('tell application "TextEdit" to set text of front document to "I have a possibility to join tomorrow."')
start = len(lines())
for i in range(10):
    activate("TextEdit"); cmd_a()
    n = len(lines())
    key(14, "option down, command down")
    wait_for(n, "critique done", 12) or wait_for(n, "capture failed", 1)
    esc(); time.sleep(0.6)
after = since(start)
crit = sum("critique done" in l for l in after)
retried = sum("retrying" in l for l in after)
doubles = [l for l in after if "double ⌘C" in l]
record("10 × ⌥⌘E, no translation", not doubles and crit == 10,
       f"critiques done {crit}/10, capture retries {retried}, double ⌘C lines {len(doubles)}")

# ---- Terminal, last ---------------------------------------------------------------------
# Secure Keyboard Entry, when on, can linger after Terminal loses focus and blind the monitor
# for whatever runs next; last, it can't spoil the other apps' results.
osa('tell application "Terminal" to do script "clear; echo \'Не будем городить огород, давай просто откатим релиз.\'; exit"')
time.sleep(2)
activate("Terminal"); cmd_a()
check_double("Terminal")
check_singles("Terminal", 12, "городить огород")
check_paste("from Terminal", "городить огород")

# ---- cleanup ----------------------------------------------------------------------------
activate("Safari"); key(13, "command down")             # ⌘W the test tab
activate("Visual Studio Code"); key(13, "command down")   # the now-empty untitled closes without a prompt
osa('tell application "Terminal" to close front window')
if not terminal_was_running:
    osa('tell application "Terminal" to quit')
osa('tell application "TextEdit" to close front document saving no')
if original_layout:
    subprocess.run([INPUTSOURCE, original_layout], capture_output=True)

print()
print(f"{sum(ok for _, ok, _ in results)}/{len(results)} checks passed")
