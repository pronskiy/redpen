import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const out = document.querySelector<HTMLElement>("#critique")!;
const status = document.querySelector<HTMLElement>("#status")!;

let buffer = "";
/** Until critique-done, the last line of the buffer may be cut off mid-word. */
let streamDone = false;
/** Whether the card is showing a critique, so a live style change knows it may redraw. */
let showingCritique = false;

type CorrectionStyle = "pen" | "inline" | "tinted";
let style: CorrectionStyle = "pen";
document.body.dataset.corrections = style;

/** Everything here is model output quoting the user's selection — never trust it as HTML. */
const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

type Note = { quote: string; tell: string; natives: string[]; nativeDone: boolean };

/**
 * Parse the prompt's own output shape rather than rendering generic markdown: the layout
 * decision needs the quote and its rewrite as separate values, which a markdown renderer
 * flattens away.
 *
 *   **Reads as:** <verdict>
 *   > "<fragment>"
 *   **Tell** — <mechanism>
 *   **Native** — "<version>" · "<alternative>"
 *
 * `nativeDone` holds the diff back while the rewrite is still streaming. Diffed half-way,
 * "requires an" against the quote "requires RFC" strikes RFC for a few frames and then takes
 * it back. The last line counts as finished once the stream has ended.
 */
function parse(md: string, done: boolean): { verdict: string; notes: Note[] } {
  let verdict = "";
  const notes: Note[] = [];
  let cur: Note | null = null;

  const lines = md.split("\n");
  for (let idx = 0; idx < lines.length; idx++) {
    const line = lines[idx];
    const v = line.match(/^\*\*Reads as:\*\*\s*(.*)/);
    if (v) { verdict = v[1]; continue; }

    if (line.startsWith(">")) {
      const q = line.replace(/^>\s*/, "").replace(/^"|"$/g, "");
      cur = { quote: q, tell: "", natives: [], nativeDone: false };
      notes.push(cur);
      continue;
    }
    const t = line.match(/^\*\*Tell\*\*\s*[—–-]\s*(.*)/);
    if (t && cur) { cur.tell = t[1]; continue; }

    const n = line.match(/^\*\*Native\*\*\s*[—–-]\s*(.*)/);
    if (n && cur) {
      cur.natives = n[1].split("·").map((x) => x.trim().replace(/^"|"$/g, "")).filter(Boolean);
      cur.nativeDone = done || idx < lines.length - 1;
      continue;
    }
    // A wrapped Tell line continues the previous one.
    if (cur && cur.tell && cur.natives.length === 0 && line.trim()) cur.tell += " " + line.trim();
  }
  return { verdict, notes };
}

const normalize = (w: string) => w.toLowerCase().replace(/[.,!?;:"“”'’]/g, "");

/**
 * Word-level alignment by longest common subsequence: the pairs [i, j] where quote word i
 * survives as rewrite word j.
 *
 * Prefix/suffix trimming was not enough: it reports one contiguous span, so any rewrite
 * that touches two separate places marks everything between them as changed. LCS keeps
 * every word that survived — which is the whole point here. Seeing *one* word struck and
 * *one* added is a rule you can carry to the next message; a wall of green is not.
 *
 * Sentence-length inputs, so the O(n·m) table costs nothing.
 */
function align(a: string[], b: string[]): [number, number][] {
  const n = a.length, m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = normalize(a[i]) === normalize(b[j])
        ? dp[i + 1][j + 1] + 1
        : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const pairs: [number, number][] = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (normalize(a[i]) === normalize(b[j])) { pairs.push([i, j]); i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) i++;
    else j++;
  }
  return pairs;
}

/** Curly against straight quotes is the model's typography, not a correction. */
const canon = (w: string) => w.replace(/[’‘]/g, "'").replace(/[“”]/g, '"');
const PUNCT = /^[.,!?;:"'—–-]+$/;

type Hunk = { del: string[]; ins: string[] };
type Op = { kind: "plain"; html: string } | ({ kind: "hunk" } & Hunk);

/**
 * Fold the quote and its rewrite into one line, in reading order: the words that stay, and
 * hunks where a struck run is followed straight away by whatever replaces it. Read the plain
 * words with the red and you have what was written; with the green, what a native would write.
 *
 * Also returns how many quote words stayed, for the caller's is-this-even-an-edit check.
 */
function merge(a: string[], b: string[]): { ops: Op[]; kept: number } | null {
  const pairs = align(a, b);
  if (!pairs.length) return null;

  const ops: Op[] = [];
  let kept = pairs.length;

  const plain = (html: string) => ops.push({ kind: "plain", html });
  const context = (words: string[]) => { kept += words.length; words.forEach((w) => plain(esc(w))); };
  const hunk = (del: string[], ins: string[]) => {
    if (!del.length && !ins.length) return;
    const prev = ops[ops.length - 1];
    if (prev?.kind === "hunk") { prev.del.push(...del); prev.ins.push(...ins); }
    else ops.push({ kind: "hunk", del: [...del], ins: [...ins] });
  };

  // Words that matched once normalised can still differ. Case ("i" -> "I") is a real fix and
  // swaps the whole word; punctuation colours only the mark itself ("up" -> "up,").
  const keep = (x: string, y: string, endOfRewrite: boolean) => {
    const cx = canon(x), cy = canon(y);
    if (cx === cy) return plain(esc(x));
    let p = 0;
    while (p < cx.length && p < cy.length && cx[p] === cy[p]) p++;
    let s = 0;
    while (s < cx.length - p && s < cy.length - p && cx[cx.length - 1 - s] === cy[cy.length - 1 - s]) s++;
    const removed = cx.slice(p, cx.length - s), added = cy.slice(p, cy.length - s);
    if (!PUNCT.test(removed || ".") || !PUNCT.test(added || ".")) return hunk([x], [y]);
    // A fragment that stops before the full stop didn't remove it.
    if (endOfRewrite && !added && s === 0) return plain(esc(x));
    plain(
      esc(cx.slice(0, p)) +
      (removed ? `<del class="punct">${esc(removed)}</del>` : "") +
      (added ? `<ins class="punct">${esc(added)}</ins>` : "") +
      esc(cx.slice(cx.length - s))
    );
  };

  // The rewrite fixes a *fragment*, and the prompt asks for the shortest one that works, so
  // it routinely stops before the end of the quoted span: "Great to see what's cooking in
  // Qodana" comes back as "It's great to see". LCS has nothing to align the four trailing
  // words with and calls them deleted, so the card strikes text the critique never objected
  // to — the loudest possible way to be wrong.
  //
  // A rewrite that simply ran out is recognisable: the word at its own end is one that
  // matched, so it never reached past there and the quote words beyond it are context. When
  // the rewrite does carry words of its own out there it genuinely replaced them ("previos
  // submissions" -> "previous submissions"), and the strike stays. So does a rewrite that
  // ended the sentence on its last word ("this regard?" -> "this?"): it dropped the rest on
  // purpose.
  const [li, lj] = pairs[pairs.length - 1];
  const endsSentence = /[.?!]$/.test(b[lj]) && !/[.?!]$/.test(a[li]);
  const leadingContext = pairs[0][1] === 0;
  const trailingContext = lj === b.length - 1 && !endsSentence;

  let pi = 0, pj = 0;
  pairs.forEach(([i, j], k) => {
    if (k === 0 && leadingContext) context(a.slice(0, i));
    else hunk(a.slice(pi, i), b.slice(pj, j));
    keep(a[i], b[j], j === b.length - 1);
    pi = i + 1; pj = j + 1;
  });
  if (trailingContext) context(a.slice(pi));
  else hunk(a.slice(pi), b.slice(pj));

  return { ops, kept };
}

/**
 * `pen` writes the correction above what it replaces, the way a teacher marks a page, and a
 * bare insertion gets a caret in the gap. `inline` and `tinted` keep the line flat: struck,
 * then the fix — they differ only in CSS.
 */
function hunkHtml(h: Hunk): string {
  const d = esc(h.del.join(" ")), i = esc(h.ins.join(" "));
  if (style === "pen") {
    // The annotation sits in an inline-block so ruby layout can't justify its words apart
    // across a wider base.
    if (d && i) return `<ruby><del>${d}</del><rt><span>${i}</span></rt></ruby>`;
    if (i) return `<ruby><span class="caret">‸</span><rt><span>${i}</span></rt></ruby>`;
    return `<del>${d}</del>`;
  }
  return [d && `<del>${d}</del>`, i && `<ins>${i}</ins>`].filter(Boolean).join(" ");
}

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

  // No rewrite, or not all of it yet: the quote as written. The marks land once, together.
  if (!native || !note.nativeDone) {
    return `
      <li class="note">
        <p class="quote">${esc(note.quote)}</p>
        <p class="tell">${esc(note.tell)}</p>
      </li>`;
  }

  const aw = note.quote.split(/\s+/).filter(Boolean);
  const bw = native.split(/\s+/).filter(Boolean);
  const merged = merge(aw, bw);

  if (!merged || merged.kept / aw.length < MIN_KEPT_SHARE) {
    // Too little in common to call it an edit — no strikethrough, no green.
    return `
      <li class="note stacked">
        <p class="quote">${esc(note.quote)}</p>
        <p class="tell">${esc(note.tell)}</p>
        <p class="native"><span class="elbow">→</span> ${esc(native)}${altHtml}</p>
      </li>`;
  }

  const line = merged.ops.map((op) => (op.kind === "hunk" ? hunkHtml(op) : op.html)).join(" ");
  return `
    <li class="note">
      <p class="quote">${line}</p>
      <p class="tell">${esc(note.tell)}${altHtml}</p>
    </li>`;
}

function render() {
  const { verdict, notes } = parse(buffer, streamDone);
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

type UiSettings = { font_size: number; correction_style: string };

// Size and correction style come from config.json and hot-reload with everything else, so
// they can be tuned live rather than rebuilt. Everything in the stylesheet is in rem, so
// setting the root scales the whole card.
function applySettings(s: UiSettings) {
  if (s.font_size >= 8 && s.font_size <= 40) document.documentElement.style.fontSize = `${s.font_size}px`;
  // A typo in the config draws the default rather than nothing.
  style = s.correction_style === "inline" || s.correction_style === "tinted" ? s.correction_style : "pen";
  document.body.dataset.corrections = style;
  if (showingCritique) render();
}
invoke<UiSettings>("ui_settings").then(applySettings).catch(() => {});
listen<UiSettings>("ui-settings", (e) => applySettings(e.payload));

listen("critique-start", () => {
  buffer = "";
  streamDone = false;
  showingCritique = true;
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

listen<string>("critique-done", () => {
  setStatus("");
  // The final Native line often has no newline after it, so it is only diffed now.
  streamDone = true;
  render();
});
listen<string>("critique-error", (e) => setStatus(e.payload, "error"));

// C1.2: the trailing ```json block is stripped in Rust, so it arrives separately and is
// never rendered (decision #6). Epic E is what finally stores these.
listen<string[]>("critique-tags", (e) => console.debug("[redpen] tags", e.payload));

// ---- updates (tray → "Check for Updates…") --------------------------------------------
// Rust owns the flow and emits; this only draws. Same split as critique, so the panel has
// exactly one place that knows what a card looks like.

type Update = { version: string; current: string; notes: string | null; unsigned: boolean };

/** The panel is the only surface this app has, so an update reuses the card wholesale. */
function updateCard(title: string, body: string, extra = "") {
  showingCritique = false;
  out.innerHTML = `
    <p class="verdict">${esc(title)}</p>
    <div class="note stacked">
      <p class="tell">${body}</p>
      ${extra}
    </div>`;
}

listen("update-checking", () => {
  setStatus("");
  updateCard("Checking for updates", "Asking the release endpoint what the latest build is.");
});

listen<string>("update-none", (e) =>
  updateCard("You're up to date", `redpen ${esc(e.payload)} is the latest build.`));

listen<string>("update-error", (e) =>
  updateCard("Couldn't check for updates", esc(e.payload)));

listen<Update>("update-available", (e) => {
  const u = e.payload;
  const notes = u.notes ? `<p class="tell">${esc(u.notes)}</p>` : "";
  // The warning is the whole reason this flow is not just a silent auto-update: on a build
  // macOS cannot tie to a stable signing identity, replacing the binary drops the
  // Accessibility grant and ⌥⌘E goes quiet with no visible cause.
  const warning = u.unsigned
    ? `<p class="tell"><strong>This build isn't signed with a certificate.</strong> Installing
         will replace the binary, and macOS will revoke redpen's Accessibility permission —
         you'll have to grant it again before capture works.</p>`
    : "";
  updateCard(
    "Update available",
    `redpen ${esc(u.version)} is out. You have ${esc(u.current)}.`,
    `${notes}${warning}<button id="do-update">Install and restart</button>`
  );
  out.querySelector("#do-update")?.addEventListener("click", () => {
    updateCard("Downloading…", "redpen will restart when the update is in place.");
    invoke("install_update").catch((err) => updateCard("Update failed", esc(String(err))));
  });
});

listen<number>("update-progress", (e) =>
  updateCard("Downloading…", `${e.payload}% — redpen will restart when the update is in place.`));

// C1.4: Accessibility is what lets capture synthesise ⌘C. Without it every press fails, so
// say so once, up front, rather than once per press.
listen("needs-accessibility", () => {
  setStatus("");
  showingCritique = false;
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
