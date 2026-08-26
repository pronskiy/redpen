#!/usr/bin/env bash
#
# redpen prompt eval harness — SPEC.md phase A3.
#
# Runs one prompt variant across the corpus, machine-checks the tag block,
# and writes a rating sheet for the human pass. No Tauri, no Rust, no app.
#
#   evals/run.sh                          # prompts/critique.md over docs/corpus
#   evals/run.sh -p prompts/v2.md         # a variant
#   evals/run.sh -e medium                # sweep reasoning effort
#   evals/run.sh -n                       # resolve config + list corpus, spend nothing
#
set -uo pipefail

PROMPT_FILE=""
CORPUS_DIR="docs/corpus"
EFFORT=""
MODEL=""
CONCURRENCY=4
MAX_TOKENS=16000
DRY_RUN=0
REASSEMBLE=0
CONFIG_JSON="$HOME/Library/Application Support/redpen/config.json"

while getopts "p:c:e:m:j:o:nRh" opt; do
  case "$opt" in
    p) PROMPT_FILE="$OPTARG" ;;
    c) CORPUS_DIR="$OPTARG" ;;
    e) EFFORT="$OPTARG" ;;
    m) MODEL="$OPTARG" ;;
    j) CONCURRENCY="$OPTARG" ;;
    o) OUT_FILE="$OPTARG" ;;
    n) DRY_RUN=1 ;;
    R) REASSEMBLE=1 ;;
    h) sed -n '3,12p' "$0"; exit 0 ;;
    *) exit 2 ;;
  esac
done

command -v jq   >/dev/null || { echo "need jq: brew install jq" >&2; exit 1; }
command -v curl >/dev/null || { echo "need curl" >&2; exit 1; }

# ---- config resolution -------------------------------------------------------
# Same precedence the app will use, and the same file, so tuning here exercises
# the config contract (SPEC A2.1) before config.rs exists.
cfg() {
  [ -f "$CONFIG_JSON" ] || { echo ""; return; }
  jq -r --arg k "$1" '.[$k] // empty' "$CONFIG_JSON" 2>/dev/null
}
# First run: write the config the app will use, rather than making you export a key into
# every shell. Prototypes what A2.1 does in Rust; `system_prompt_path` points at the repo so
# there is never a second copy of the prompt to drift out of sync.
bootstrap_config() {
  [ -f "$CONFIG_JSON" ] && return 0
  mkdir -p "$(dirname "$CONFIG_JSON")" || return 1
  jq -n --arg pp "$PWD/prompts/critique.md" '{
    api_key: "",
    base_url: "https://api.anthropic.com",
    model: "claude-opus-5",
    effort: "low",
    hotkey: "Alt+Cmd+E",
    system_prompt_path: $pp
  }' > "$CONFIG_JSON" || return 1
  chmod 600 "$CONFIG_JSON"
  echo "created $CONFIG_JSON — see config.example.jsonc for what each field means"
}
bootstrap_config

API_KEY="${ANTHROPIC_API_KEY:-$(cfg api_key)}"
BASE_URL="${ANTHROPIC_BASE_URL:-$(cfg base_url)}"
[ -n "$BASE_URL" ] || BASE_URL="https://api.anthropic.com"
BASE_URL="${BASE_URL%/}"
[ -n "$MODEL" ] || MODEL="$(cfg model)"
[ -n "$MODEL" ] || MODEL="claude-opus-5"
[ -n "$EFFORT" ] || EFFORT="$(cfg effort)"
[ -n "$EFFORT" ] || EFFORT="low"
# No -p given: use the prompt the app itself is pointed at, so tuning matches production.
[ -n "$PROMPT_FILE" ] || PROMPT_FILE="$(cfg system_prompt_path)"
[ -n "$PROMPT_FILE" ] || PROMPT_FILE="prompts/critique.md"
[ -f "$PROMPT_FILE" ] || { echo "no prompt file: $PROMPT_FILE" >&2; exit 1; }

case "$MODEL" in
  claude-opus-5*|claude-fable-5*|claude-mythos-5*) SUPPORTS_FALLBACKS=1 ;;
  *) SUPPORTS_FALLBACKS=0 ;;
esac

PROMPT_TAG="$(basename "$PROMPT_FILE" .md)"
OUT_FILE="${OUT_FILE:-$CORPUS_DIR/results-$PROMPT_TAG-$EFFORT.md}"
WORK="evals/.out/$PROMPT_TAG-$EFFORT"

# Corpus files, sorted. README.md is instructions, not a specimen.
CORPUS=()
while IFS= read -r f; do CORPUS+=("$f"); done < <(
  find "$CORPUS_DIR" -maxdepth 1 -name '*.md' \
    ! -name 'README.md' ! -name 'results-*.md' | sort
)

# Tag vocabulary: extracted from the prompt itself, never duplicated here.
VOCAB="$(sed -n '/TAGS:BEGIN/,/TAGS:END/p' "$PROMPT_FILE" | sed '1d;$d' | sed '/^[[:space:]]*$/d')"

echo "prompt      $PROMPT_FILE"
echo "model       $MODEL"
echo "effort      $EFFORT"
echo "base_url    $BASE_URL"
echo "api_key     $([ -n "$API_KEY" ] && echo "resolved (${#API_KEY} chars)" || echo "MISSING")"
echo "corpus      ${#CORPUS[@]} texts in $CORPUS_DIR"
echo "vocabulary  $(echo "$VOCAB" | wc -l | tr -d ' ') tags"
echo "fallbacks   $([ "$SUPPORTS_FALLBACKS" = "1" ] && echo "on (refusal safety net)" || echo "off — $MODEL does not support the parameter")"
echo "out         $OUT_FILE"

if [ "${#CORPUS[@]}" -eq 0 ]; then
  echo >&2
  echo "Corpus is empty. See $CORPUS_DIR/README.md — this is the step that needs a human." >&2
  exit 1
fi
[ "$DRY_RUN" -eq 1 ] && { printf '%s\n' "${CORPUS[@]}"; exit 0; }
if [ -z "$API_KEY" ]; then
  echo >&2
  echo "No API key yet. Add it to the config the app will also read:" >&2
  echo >&2
  echo "  c=\"$CONFIG_JSON\"" >&2
  echo "  jq '.api_key = \"sk-ant-...\"' \"\$c\" > \"\$c.tmp\" && mv \"\$c.tmp\" \"\$c\"" >&2
  echo >&2
  echo "or export ANTHROPIC_API_KEY for a one-off run." >&2
  exit 1
fi

mkdir -p "$WORK"

# ---- one call ----------------------------------------------------------------
call_one() {
  local src="$1" base out started elapsed
  base="$(basename "$src" .md)"
  out="$WORK/$base"
  started=$(date +%s)
  jq -n --rawfile sys "$PROMPT_FILE" --rawfile txt "$src" \
        --arg model "$MODEL" --arg effort "$EFFORT" --argjson mt "$MAX_TOKENS" \
        --argjson fb "$SUPPORTS_FALLBACKS" '{
    model: $model,
    max_tokens: $mt,
    system: $sys,
    messages: [{role: "user", content: $txt}],
    thinking: {type: "adaptive"},
    output_config: {effort: $effort}
  } + (if $fb == 1 then {fallbacks: "default"} else {} end)' > "$out.req.json"
  local hdrs=(-H "x-api-key: $API_KEY" -H "anthropic-version: 2023-06-01" -H "content-type: application/json")
  [ "$SUPPORTS_FALLBACKS" = "1" ] && hdrs+=(-H "anthropic-beta: server-side-fallback-2026-07-01")
  curl -sS --max-time 300 "$BASE_URL/v1/messages" "${hdrs[@]}" \
    -d @"$out.req.json" > "$out.res.json" 2>"$out.err"
  elapsed=$(( $(date +%s) - started ))
  echo "$elapsed" > "$out.secs"
  printf '  %-28s %3ss\n' "$base" "$elapsed"
}

if [ "$REASSEMBLE" -eq 1 ]; then
  echo
  echo "reassembling from $WORK — no API calls"
  [ -d "$WORK" ] || { echo "nothing cached at $WORK; run without -R first" >&2; exit 1; }
else
  echo
  echo "running ${#CORPUS[@]} calls, $CONCURRENCY at a time..."
  i=0
  for src in "${CORPUS[@]}"; do
    call_one "$src" &
    i=$((i + 1))
    [ $((i % CONCURRENCY)) -eq 0 ] && wait
  done
  wait
fi

# ---- assemble ----------------------------------------------------------------
# Pull the LAST ```json fence out of the critique — that is the tag block.
last_json_fence() {
  awk '
    /^```json[[:space:]]*$/ { inb=1; buf=""; next }
    inb && /^```[[:space:]]*$/ { inb=0; last=buf; next }
    inb { buf = buf $0 "\n" }
    END { printf "%s", last }
  ' "$1"
}

ok_parse=0; ok_vocab=0; total=0; declare -a ROWS=()

{
  echo "# Corpus results — \`$PROMPT_TAG\` @ effort \`$EFFORT\`"
  echo
  echo "Model \`$MODEL\` · generated by \`evals/run.sh\` · **ratings below are blank on purpose — fill them in.**"
  echo
  echo "## Rubric"
  echo
  echo "Rate each output with exactly one label. Decide the label from the *whole* output."
  echo
  echo "- **useful** — names at least one thing you had not noticed was off, and the natural"
  echo "  version is one you would actually have sent. It would have changed the text."
  echo "- **correctly-silent** — the text really was fine and the critique said so instead of"
  echo "  manufacturing something. Counts as a pass: on a corpus of real writing several texts"
  echo "  have nothing wrong with them, and restraint on those is the behaviour you want."
  echo "- **water** — accurate but generic. Praise, restatement, or advice that would fit any"
  echo "  other text. Nothing actionable about *this* one."
  echo "- **wrong** — flags correct English as an error, proposes a rewrite that is worse or"
  echo "  changes your meaning, or misdiagnoses the cause."
  echo
  echo "\`wrong items\` is a separate count of individual bad fragments, diagnostic only — the"
  echo "SPEC guardrail is ≥ 15/20 rated **useful** or **correctly-silent**. Track wrong items"
  echo "anyway: a prompt that scores useful while misteaching twice per run is not shippable."
  echo
  echo "## Summary"
  echo
  echo "| # | text | secs | frags | tag block | tags | rating | wrong items |"
  echo "|---|------|------|-------|-----------|------|--------|-------------|"
} > "$OUT_FILE"

n=0
for src in "${CORPUS[@]}"; do
  base="$(basename "$src" .md)"; out="$WORK/$base"; n=$((n + 1)); total=$((total + 1))
  secs="$(cat "$out.secs" 2>/dev/null || echo '?')"

  if jq -e '.type == "error"' "$out.res.json" >/dev/null 2>&1; then
    err="$(jq -r '.error.message // "unknown"' "$out.res.json")"
    echo "| $n | \`$base\` | $secs | — | ❌ API error | — | | |" >> "$OUT_FILE"
    ROWS+=("$n|$base|API ERROR: $err|")
    continue
  fi

  jq -r '[.content[]? | select(.type=="text") | .text] | join("")' "$out.res.json" > "$out.txt" 2>/dev/null
  stop="$(jq -r '.stop_reason // "?"' "$out.res.json")"
  last_json_fence "$out.txt" > "$out.tags.json"

  if jq -e 'type=="object" and (.tags|type=="array") and (all(.tags[]; type=="string"))' \
       "$out.tags.json" >/dev/null 2>&1; then
    parse="✅"; ok_parse=$((ok_parse + 1))
    tags="$(jq -r '.tags | join(", ")' "$out.tags.json")"
    bad="$(jq -r '.tags[]' "$out.tags.json" | grep -vxF "$VOCAB" | sort -u | tr '\n' ' ')"
    if [ -z "$(echo "$bad" | tr -d ' ')" ]; then ok_vocab=$((ok_vocab + 1)); else parse="⚠️ off-vocab: $bad"; fi
    [ -z "$tags" ] && tags="_(none)_"
  else
    parse="❌ no parse"; tags="—"
  fi
  frags="$(grep -c '^> "' "$out.txt" 2>/dev/null)"; frags="${frags:-0}"
  ntags="$(jq -r '.tags | length' "$out.tags.json" 2>/dev/null)"; ntags="${ntags:-0}"
  frags_cell="$frags"
  if [ "$ntags" -lt "$frags" ] 2>/dev/null; then
    frags_cell="$frags ⚠️"
    parse="$parse ⚠️ $((frags - ntags)) untagged"
  fi
  [ "$stop" = "max_tokens" ] && parse="$parse ⚠️ truncated"
  [ "$stop" = "refusal" ] && parse="❌ refusal"

  echo "| $n | \`$base\` | $secs | $frags_cell | $parse | $tags | | |" >> "$OUT_FILE"
done

{
  echo
  echo "## Outputs"
  echo
} >> "$OUT_FILE"

n=0
for src in "${CORPUS[@]}"; do
  base="$(basename "$src" .md)"; out="$WORK/$base"; n=$((n + 1))
  {
    echo "### $n. \`$base\`"
    echo
    echo "**Rating:** _(useful / correctly-silent / water / wrong)_ · **Wrong items:** _( )_ · **Notes:**"
    echo
    echo "<details><summary>original text</summary>"
    echo
    echo '```'
    cat "$src"
    echo '```'
    echo
    echo "</details>"
    echo
    if [ -s "$out.txt" ]; then cat "$out.txt"; else echo "_(no output — see $out.res.json)_"; fi
    echo
    echo "---"
    echo
  } >> "$OUT_FILE"
done

echo
echo "─────────────────────────────────────────────"
echo "structure guardrail   tag block parses:  $ok_parse/$total"
echo "                      tags in vocabulary: $ok_vocab/$total"
echo "                      (SPEC A3 requires $total/$total)"
echo "usefulness guardrail  needs your ratings in $OUT_FILE"
echo "─────────────────────────────────────────────"
