import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const out = document.querySelector<HTMLElement>("#critique")!;
const status = document.querySelector<HTMLElement>("#status")!;

let buffer = "";

/** Everything here is model output quoting the user's selection — never trust it as HTML. */
const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

type Note = { quote: string; tell: string; natives: string[] };

/**
 * Parse the prompt's own output shape rather than rendering generic markdown: the layout
 * decision needs the quote and its rewrite as separate values, which a markdown renderer
 * flattens away.
 *
 *   **Reads as:** <verdict>
 *   > "<fragment>"
 *   **Tell** — <mechanism>
 *   **Native** — "<version>" · "<alternative>"
 */
function parse(md: string): { verdict: string; notes: Note[] } {
  let verdict = "";
  const notes: Note[] = [];
  let cur: Note | null = null;

  for (const line of md.split("\n")) {
    const v = line.match(/^\*\*Reads as:\*\*\s*(.*)/);
    if (v) { verdict = v[1]; continue; }

    if (line.startsWith(">")) {
      const q = line.replace(/^>\s*/, "").replace(/^"|"$/g, "");
      cur = { quote: q, tell: "", natives: [] };
      notes.push(cur);
      continue;
    }
    const t = line.match(/^\*\*Tell\*\*\s*[—–-]\s*(.*)/);
    if (t && cur) { cur.tell = t[1]; continue; }

    const n = line.match(/^\*\*Native\*\*\s*[—–-]\s*(.*)/);
    if (n && cur) {
      cur.natives = n[1].split("·").map((x) => x.trim().replace(/^"|"$/g, "")).filter(Boolean);
      continue;
    }
    // A wrapped Tell line continues the previous one.
    if (cur && cur.tell && cur.natives.length === 0 && line.trim()) cur.tell += " " + line.trim();
  }
  return { verdict, notes };
}

const normalize = (w: string) => w.toLowerCase().replace(/[.,!?;:"“”'’]/g, "");

/**
 * Trim the common prefix and suffix; whatever is left is the edit.
 *
 * The layout hinges on this. Measured against the corpus, 46% of rewrites change one or
 * two words — inline annotation is perfect there. 21% restructure the sentence entirely,
 * where an inline marker would highlight most of the line and explain nothing. So the
 * span width picks the layout instead of one shape being forced on both.
 */
function editSpan(a: string, b: string) {
  const aw = a.split(/\s+/).filter(Boolean);
  const bw = b.split(/\s+/).filter(Boolean);
  let s = 0;
  while (s < aw.length && s < bw.length && normalize(aw[s]) === normalize(bw[s])) s++;
  let ae = aw.length, be = bw.length;
  while (ae > s && be > s && normalize(aw[ae - 1]) === normalize(bw[be - 1])) { ae--; be--; }
  return { aw, bw, s, ae, be, changed: Math.max(ae - s, be - s) };
}

const INLINE_MAX_WORDS = 2;

function renderNote(note: Note): string {
  const native = note.natives[0] ?? "";
  const alt = note.natives[1];
  const altHtml = alt ? ` <span class="alt">or “${esc(alt)}”</span>` : "";

  if (native) {
    const d = editSpan(note.quote, native);
    if (d.changed > 0 && d.changed <= INLINE_MAX_WORDS) {
      const before = d.aw.slice(0, d.s).join(" ");
      const hit = d.aw.slice(d.s, d.ae).join(" ");
      const after = d.aw.slice(d.ae).join(" ");
      const fix = d.bw.slice(d.s, d.be).join(" ");
      return `
        <div class="note inline">
          <p class="quote">${esc(before)} <mark>${esc(hit)}</mark> ${esc(after)}</p>
          <p class="fix"><span class="elbow">└</span> <strong>${esc(fix || "—")}</strong>
             <span class="tell">${esc(note.tell)}</span>${altHtml}</p>
        </div>`;
    }
  }
  // Restructured: show it whole, because there is no single word to point at.
  return `
    <div class="note stacked">
      <p class="quote">${esc(note.quote)}</p>
      <p class="tell">${esc(note.tell)}</p>
      ${native ? `<p class="native"><span class="elbow">→</span> ${esc(native)}${altHtml}</p>` : ""}
    </div>`;
}

function render() {
  const { verdict, notes } = parse(buffer);
  out.innerHTML =
    (verdict ? `<p class="verdict">${esc(verdict)}</p>` : "") +
    notes.map(renderNote).join("");
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
// card would sit blank for the whole thinking phase and look broken.
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

// C1.2: the trailing ```json block is stripped in Rust, so it arrives separately and is
// never rendered (decision #6). Epic E is what finally stores these.
listen<string[]>("critique-tags", (e) => console.debug("[redpen] tags", e.payload));

// C1.4: Accessibility is what lets capture synthesise ⌘C. Without it every press fails, so
// say so once, up front, rather than once per press.
listen("needs-accessibility", () => {
  setStatus("");
  out.innerHTML = `
    <p class="verdict">One permission first</p>
    <div class="note stacked">
      <p class="tell">redpen copies your selection by pressing ⌘C for you, and macOS only
        allows that with <strong>Accessibility</strong> permission.</p>
      <p class="tell">macOS ties it to the exact binary, so a rebuild or an update can
        revoke it. If capture ever goes quiet, check here first.</p>
      <button id="open-ax">Open Accessibility settings</button>
    </div>`;
  out.querySelector("#open-ax")?.addEventListener("click", () => {
    invoke("open_accessibility_settings").catch(() => {});
  });
});

// Dismissal lives in Rust (B1.4). The panel can never become key, so no keydown reaches
// this webview — a listener here would be silent code.
