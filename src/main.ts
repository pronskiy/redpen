import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";

const out = document.querySelector<HTMLElement>("#critique")!;
const status = document.querySelector<HTMLElement>("#status")!;

let buffer = "";

/**
 * The critique quotes the user's selection verbatim, and that selection can be anything
 * they highlighted — including markup copied off a web page. Rendering it as HTML in a
 * webview that holds Tauri IPC would be an injection path straight to the Rust side.
 *
 * Escaping `&` and `<` closes it. Deliberately NOT `>`: the prompt's whole output format
 * is blockquoted fragments, and escaping `>` would break every one of them. `>` alone
 * cannot open a tag.
 */
function neutralizeTags(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
}

function render() {
  out.innerHTML = marked.parse(neutralizeTags(buffer), { async: false }) as string;
  out.scrollTop = out.scrollHeight;
}

function setStatus(text: string, kind: "" | "error" = "") {
  status.textContent = text;
  status.dataset.kind = kind;
}

listen("critique-start", () => {
  buffer = "";
  out.innerHTML = "";
  setStatus("reading…");
});

// Thinking output is hidden by default on these models (decision #16), so without this the
// panel would sit blank for the whole thinking phase and look broken.
listen("critique-thinking", () => {
  if (!buffer) setStatus("thinking…");
});

listen<string>("critique-delta", (e) => {
  if (!buffer) setStatus("");
  buffer += e.payload;
  render();
});

listen<string>("critique-done", () => setStatus(""));

listen<string>("critique-error", (e) => setStatus(e.payload, "error"));

// C1.2: the trailing ```json tag block is stripped in Rust and arrives here separately, so
// the panel only ever renders prose (decision #6). Epic E is what finally stores these.
listen<string[]>("critique-tags", (e) => {
  console.debug("[redpen] tags", e.payload);
});

// C1.4: Accessibility is what lets capture synthesise ⌘C. Without it every press fails, so
// say so once, up front, rather than once per press.
listen("needs-accessibility", () => {
  setStatus("");
  out.innerHTML = `
    <h2>One permission first</h2>
    <p>redpen copies your selection by pressing ⌘C for you, and macOS only allows that
       with <strong>Accessibility</strong> permission.</p>
    <p>Open <em>Privacy &amp; Security → Accessibility</em> and switch redpen on. No restart
       needed — press the hotkey again once it is granted.</p>
    <p class="aside">macOS ties this to the exact binary, so a rebuild or an app update can
       revoke it. If capture ever goes quiet, check here first.</p>
    <button id="open-ax">Open Accessibility settings</button>`;
  out.querySelector("#open-ax")?.addEventListener("click", () => {
    invoke("open_accessibility_settings").catch(() => {});
  });
});

// Dismissal moved to the Rust side in B1.4. The panel is now a non-activating NSPanel and
// can never become key, so no keydown ever reaches this webview — a listener here would be
// silent code. Esc, click-outside and app-switch are all observed by NSEvent monitors in
// panel.rs. The `dismiss` command stays available for a close button in C1.
