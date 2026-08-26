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

// Dismissal moved to the Rust side in B1.4. The panel is now a non-activating NSPanel and
// can never become key, so no keydown ever reaches this webview — a listener here would be
// silent code. Esc, click-outside and app-switch are all observed by NSEvent monitors in
// panel.rs. The `dismiss` command stays available for a close button in C1.
