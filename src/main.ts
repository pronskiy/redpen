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
 * Word-level diff by longest common subsequence.
 *
 * Prefix/suffix trimming was not enough: it reports one contiguous span, so any rewrite
 * that touches two separate places marks everything between them as changed. LCS keeps
 * every word that survived — which is the whole point here. Seeing *one* word struck and
 * *one* added is a rule you can carry to the next message; a wall of green is not.
 *
 * Sentence-length inputs, so the O(n·m) table costs nothing.
 */
function wordDiff(a: string[], b: string[]) {
  const n = a.length, m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = normalize(a[i]) === normalize(b[j])
        ? dp[i + 1][j + 1] + 1
        : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const kept: boolean[] = new Array(n).fill(false);
  const added: boolean[] = new Array(m).fill(true);
  let i = 0, j = 0, changed = 0;
  while (i < n && j < m) {
    if (normalize(a[i]) === normalize(b[j])) { kept[i] = true; added[j] = false; i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { i++; changed++; }
    else { j++; changed++; }
  }
  changed += (n - i) + (m - j);

  // The rewrite fixes a *fragment*, and the prompt asks for the shortest one that works, so
  // it routinely stops before the end of the quoted span: "Great to see what's cooking in
  // Qodana" comes back as "It's great to see". LCS has nothing to align the four trailing
  // words with and calls them deleted, so the card strikes text the critique never objected
  // to — the loudest possible way to be wrong.
  //
  // A rewrite that simply ran out is recognisable: the word at its own end is one that
  // matched, so it never reached past there and the quote words beyond it are context. When
  // the rewrite does carry words of its own out there it genuinely replaced them ("previos
  // submissions" -> "previous submissions"), and the strike stays.
  const first = kept.indexOf(true), last = kept.lastIndexOf(true);
  if (first !== -1) {
    if (!added[0]) for (let k = 0; k < first; k++) { kept[k] = true; changed--; }
    if (!added[m - 1]) for (let k = last + 1; k < n; k++) { kept[k] = true; changed--; }
  }

  return { kept, added, changed };
}

/** Struck-through where a word was dropped, plain everywhere else. */
function markRemoved(words: string[], kept: boolean[]): string {
  return words
    .map((w, i) => (kept[i] ? esc(w) : `<del>${esc(w)}</del>`))
    .join(" ");
}

/** Green only on what is genuinely new — everything you already had stays neutral. */
function markAdded(words: string[], added: boolean[]): string {
  return words
    .map((w, i) => (added[i] ? `<ins>${esc(w)}</ins>` : esc(w)))
    .join(" ");
}

const INLINE_MAX_WORDS = 2;

/**
 * Below this share of surviving words, the rewrite is not an edit of the quote — it is a
 * different sentence, and diffing two unrelated strings produces noise. It happens when the
 * model quotes the wrong span (observed: quote "in a week, unless someone has any
 * objections" against rewrite "in a while"), and striking eight words to add one reads as a
 * bug even when the underlying critique is fine.
 *
 * Show both plainly instead. A diff that cannot be trusted is worse than no diff.
 */
const MIN_KEPT_SHARE = 0.3;

function renderNote(note: Note): string {
  const native = note.natives[0] ?? "";
  const alt = note.natives[1];
  const altHtml = alt ? ` <span class="alt">or “${esc(alt)}”</span>` : "";

  if (!native) {
    return `
      <li class="note stacked">
        <p class="quote">${esc(note.quote)}</p>
        <p class="tell">${esc(note.tell)}</p>
      </li>`;
  }

  const aw = note.quote.split(/\s+/).filter(Boolean);
  const bw = native.split(/\s+/).filter(Boolean);
  const d = wordDiff(aw, bw);
  const removedHtml = markRemoved(aw, d.kept);
  const addedHtml = markAdded(bw, d.added);

  // A tight edit gets the compact form: the line, then just the replacement hanging off it.
  if (d.changed > 0 && d.changed <= INLINE_MAX_WORDS) {
    const fix = bw.filter((_, i) => d.added[i]).join(" ");
    return `
      <li class="note inline">
        <p class="quote">${removedHtml}</p>
        <p class="fix"><span class="elbow">└</span> <ins>${esc(fix || "—")}</ins>
           <span class="tell">${esc(note.tell)}</span>${altHtml}</p>
      </li>`;
  }

  const keptShare = aw.length ? d.kept.filter(Boolean).length / aw.length : 1;
  if (keptShare < MIN_KEPT_SHARE) {
    // Too little in common to call it an edit — no strikethrough, no green.
    return `
      <li class="note stacked">
        <p class="quote">${esc(note.quote)}</p>
        <p class="tell">${esc(note.tell)}</p>
        <p class="native"><span class="elbow">→</span> ${esc(native)}${altHtml}</p>
      </li>`;
  }

  // A restructure needs the whole rewrite — but still only the new words in green.
  return `
    <li class="note stacked">
      <p class="quote">${removedHtml}</p>
      <p class="tell">${esc(note.tell)}</p>
      <p class="native"><span class="elbow">→</span> ${addedHtml}${altHtml}</p>
    </li>`;
}

function render() {
  const { verdict, notes } = parse(buffer);
  // An <ol> even for a single note: render() re-runs on every stream delta, so a list that
  // only appeared once the second note landed would shove the first one sideways mid-stream.
  const list = notes.length ? `<ol class="notes">${notes.map(renderNote).join("")}</ol>` : "";
  out.innerHTML =
    (verdict ? `<p class="verdict">${esc(verdict)}</p>` : "") + list;
  out.scrollTop = out.scrollHeight;
}

function setStatus(text: string, kind: "" | "error" = "") {
  status.textContent = text;
  status.dataset.kind = kind;
}

// Base size comes from config.json and hot-reloads with everything else, so it can be
// tuned live rather than rebuilt. Everything in the stylesheet is in rem, so setting the
// root scales the whole card.
function applyFontSize(px: number) {
  if (px >= 8 && px <= 40) document.documentElement.style.fontSize = `${px}px`;
}
invoke<number>("ui_settings").then(applyFontSize).catch(() => {});
listen<number>("ui-settings", (e) => applyFontSize(e.payload));

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
