import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
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

// A2.3: dismissing must abort the request, not just hide the window — otherwise tokens keep
// generating (and billing) after you have stopped looking.
//
// This works because the window is still an ordinary focusable window. Once B1 converts it
// to a non-activating NSPanel it will never be key, so Esc will land in whatever app you
// were typing in and this listener will stop firing — B1.4 replaces it with a global event
// monitor. Deliberately not solved early.
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    invoke("dismiss").catch((err) => setStatus(`dismiss failed: ${err}`, "error"));
  }
});
