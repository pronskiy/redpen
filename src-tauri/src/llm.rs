//! Streams a critique from the Anthropic Messages API.
//!
//! `evals/run.sh` is the reference implementation — it has been posting this exact body
//! since before there was any Rust, and every quirk encoded here was learned there against
//! the live API:
//!
//! * The prompt goes in the **`system` field**, never concatenated into the user message.
//! * `fallbacks` is Opus-5/Fable-5-tier only; sending it to Sonnet 5 is a hard 400
//!   (decision #24), so it is gated on the model string.
//! * `effort` is the latency lever, not the model tier (decision #16). Thinking is on by
//!   default and its output is hidden, so the panel would sit blank without a signal —
//!   hence `critique-thinking`.
//!
//! Nothing here ever writes to the user's text. Decision #1.

use serde_json::json;

/// Events the webview listens for. A2.3 renders these.
pub const EVENT_START: &str = "critique-start";
pub const EVENT_THINKING: &str = "critique-thinking";
pub const EVENT_DELTA: &str = "critique-delta";
pub const EVENT_DONE: &str = "critique-done";
pub const EVENT_TAGS: &str = "critique-tags";
pub const EVENT_ERROR: &str = "critique-error";

/// Server-side refusal fallbacks exist only on the Opus 5 / Fable 5 tier. Verified against
/// the live API: `'claude-sonnet-5' does not support the fallbacks parameter`.
fn supports_fallbacks(model: &str) -> bool {
    model.starts_with("claude-opus-5")
        || model.starts_with("claude-fable-5")
        || model.starts_with("claude-mythos-5")
}

pub fn build_body(model: &str, effort: &str, system_prompt: &str, text: &str) -> serde_json::Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 8000,
        "stream": true,
        "system": system_prompt,
        "messages": [{ "role": "user", "content": text }],
        "thinking": { "type": "adaptive" },
        "output_config": { "effort": effort },
    });
    if supports_fallbacks(model) {
        body["fallbacks"] = json!("default");
    }
    body
}

/// The response contract is prose followed by a trailing ```json tag block (decision #6).
/// The user must only ever see the prose — tags are for the Epic E journal.
///
/// Splitting here rather than in the renderer means the webview never receives the JSON at
/// all, and the tags surface in Rust, where Epic E's store will live.
///
/// The whole difficulty is that it streams. `"```json"` arrives a character at a time, so a
/// naive check renders "``" and "```js" for a frame each before they vanish. The fix is to
/// hold back any tail that *could still become* the fence.
pub const TAG_FENCE: &str = "```json";

#[derive(Default)]
pub struct ProseSplitter {
    pending: String,
    tags_raw: String,
    in_tags: bool,
}

impl ProseSplitter {
    /// Feed a chunk; get back only the text that is safe to display.
    pub fn push(&mut self, chunk: &str) -> String {
        if self.in_tags {
            self.tags_raw.push_str(chunk);
            return String::new();
        }
        self.pending.push_str(chunk);

        if let Some(i) = self.pending.find(TAG_FENCE) {
            let prose = self.pending[..i].to_string();
            self.tags_raw = self.pending[i..].to_string();
            self.in_tags = true;
            self.pending.clear();
            return prose;
        }

        // Hold back the longest suffix that is still a prefix of the fence.
        // `is_char_boundary` first: slicing a suffix that lands inside a multi-byte
        // character panics before the comparison can rule it out. The fence is ASCII, so
        // such a suffix could never match anyway — but the panic happens first.
        let hold = (1..=TAG_FENCE.len().min(self.pending.len()))
            .rev()
            .find(|&n| {
                let start = self.pending.len() - n;
                self.pending.is_char_boundary(start)
                    && TAG_FENCE.starts_with(&self.pending[start..])
            })
            .unwrap_or(0);
        let cut = self.pending.len() - hold;
        let out = self.pending[..cut].to_string();
        self.pending = self.pending[cut..].to_string();
        out
    }

    /// Anything still held back was never a fence after all.
    pub fn finish(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }

    /// The `tags` array, if the block arrived and parsed.
    pub fn tags(&self) -> Option<Vec<String>> {
        let body = self.tags_raw.trim().strip_prefix(TAG_FENCE)?;
        let body = body.split("```").next()?;
        let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
        Some(
            v.get("tags")?
                .as_array()?
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
        )
    }
}

/// What a single SSE frame meant to us.
#[derive(Debug, PartialEq, Eq)]
pub enum Chunk {
    /// Visible prose for the panel.
    Text(String),
    /// Model is thinking; its content is hidden, but the UI needs to show *something*.
    Thinking,
    /// Stream finished, carrying `stop_reason`.
    Done(String),
    /// The API reported an error mid-stream (HTTP was already 200).
    Failed(String),
    /// Keep-alives, block boundaries, anything we do not act on.
    Ignore,
}

/// Accumulates raw bytes and yields complete SSE frames.
///
/// Two bugs this exists to prevent, both invisible until they aren't:
///
/// 1. A frame is not guaranteed to arrive in one read. A single `data:` line can be split
///    across two TCP chunks, and parsing per-chunk silently drops half a word.
/// 2. **It buffers bytes, not `str`.** A multi-byte character can straddle a chunk
///    boundary, so decoding each chunk as UTF-8 as it arrives corrupts it. That is not
///    hypothetical here — the critiques quote Russian source words and the corpus is full
///    of emoji and curly quotes. Frame separators are ASCII, so a complete frame is always
///    safe to decode.
#[derive(Default)]
pub struct SseBuffer {
    buf: Vec<u8>,
}

impl SseBuffer {
    pub fn push_bytes(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(idx) = self.buf.windows(2).position(|w| w == b"\n\n") {
            let frame: Vec<u8> = self.buf.drain(..idx + 2).collect();
            frames.push(String::from_utf8_lossy(&frame).trim_end().to_string());
        }
        frames
    }

    #[cfg(test)]
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.push_bytes(chunk.as_bytes())
    }
}

/// Interpret one complete SSE frame.
pub fn parse_frame(frame: &str) -> Chunk {
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim());
        }
    }
    if data.is_empty() || data == "[DONE]" {
        return Chunk::Ignore;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) else {
        return Chunk::Ignore;
    };
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "content_block_delta" => {
            let delta = &v["delta"];
            match delta.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "text_delta" => Chunk::Text(delta["text"].as_str().unwrap_or("").to_string()),
                // Thinking text is empty under the default `display: "omitted"`, so there is
                // nothing to render — only a reason to keep the spinner up.
                "thinking_delta" | "signature_delta" => Chunk::Thinking,
                _ => Chunk::Ignore,
            }
        }
        "message_delta" => Chunk::Done(
            v["delta"]["stop_reason"].as_str().unwrap_or("end_turn").to_string(),
        ),
        "error" => Chunk::Failed(
            v["error"]["message"].as_str().unwrap_or("unknown streaming error").to_string(),
        ),
        _ => Chunk::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_split_across_chunks_is_not_lost() {
        let mut b = SseBuffer::default();
        assert!(b.push("event: content_block_delta\ndata: {\"type\":\"content_bl").is_empty());
        let frames = b.push("ock_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(parse_frame(&frames[0]), Chunk::Text("hello".into()));
    }

    #[test]
    fn a_multibyte_char_split_across_chunks_survives() {
        // "зависеть" — the critiques quote Russian source words, and a chunk boundary
        // lands wherever TCP decides. Decoding per-chunk would produce U+FFFD here.
        let full = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"зависеть\"}}\n\n";
        let bytes = full.as_bytes();
        // Split mid-character: 'з' is two bytes, so cut between them.
        let cut = bytes.iter().position(|&b| b == 0xD0).unwrap() + 1;
        let mut b = SseBuffer::default();
        assert!(b.push_bytes(&bytes[..cut]).is_empty());
        let frames = b.push_bytes(&bytes[cut..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(parse_frame(&frames[0]), Chunk::Text("зависеть".into()));
    }

    #[test]
    fn an_emoji_split_across_chunks_survives() {
        let full = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"\u{1F64C}\"}}\n\n";
        let bytes = full.as_bytes();
        let cut = bytes.iter().position(|&b| b == 0xF0).unwrap() + 2;
        let mut b = SseBuffer::default();
        b.push_bytes(&bytes[..cut]);
        let frames = b.push_bytes(&bytes[cut..]);
        assert_eq!(parse_frame(&frames[0]), Chunk::Text("\u{1F64C}".into()));
    }

    #[test]
    fn several_frames_in_one_chunk_all_come_out() {
        let mut b = SseBuffer::default();
        let raw = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"a\"}}\n\n\
                   data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"b\"}}\n\n";
        let frames = b.push(raw);
        assert_eq!(frames.len(), 2);
        assert_eq!(parse_frame(&frames[0]), Chunk::Text("a".into()));
        assert_eq!(parse_frame(&frames[1]), Chunk::Text("b".into()));
    }

    #[test]
    fn a_partial_trailing_frame_is_held_not_emitted() {
        let mut b = SseBuffer::default();
        let frames = b.push("data: {\"type\":\"ping\"}\n\ndata: {\"type\":\"cont");
        assert_eq!(frames.len(), 1, "only the complete frame");
    }

    #[test]
    fn thinking_deltas_do_not_reach_the_panel_as_text() {
        let f = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"\"}}";
        assert_eq!(parse_frame(f), Chunk::Thinking);
    }

    #[test]
    fn pings_and_block_boundaries_are_ignored() {
        assert_eq!(parse_frame("data: {\"type\":\"ping\"}"), Chunk::Ignore);
        assert_eq!(parse_frame("data: {\"type\":\"content_block_stop\",\"index\":0}"), Chunk::Ignore);
        assert_eq!(parse_frame(": keep-alive comment"), Chunk::Ignore);
    }

    #[test]
    fn stop_reason_is_surfaced() {
        let f = "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}";
        assert_eq!(parse_frame(f), Chunk::Done("end_turn".into()));
        let r = "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"}}";
        assert_eq!(parse_frame(r), Chunk::Done("refusal".into()));
    }

    #[test]
    fn a_mid_stream_error_is_surfaced() {
        let f = "data: {\"type\":\"error\",\"error\":{\"message\":\"overloaded\"}}";
        assert_eq!(parse_frame(f), Chunk::Failed("overloaded".into()));
    }

    #[test]
    fn the_prompt_goes_in_the_system_field_never_the_user_message() {
        let b = build_body("claude-sonnet-5", "medium", "SYSTEM PROMPT", "user text");
        assert_eq!(b["system"], "SYSTEM PROMPT");
        assert_eq!(b["messages"][0]["content"], "user text");
        assert!(!b["messages"][0]["content"].as_str().unwrap().contains("SYSTEM PROMPT"));
    }

    #[test]
    fn fallbacks_only_for_the_tier_that_accepts_it() {
        // Live API, verified: 'claude-sonnet-5' does not support the `fallbacks` parameter.
        assert!(build_body("claude-sonnet-5", "medium", "s", "t").get("fallbacks").is_none());
        assert!(build_body("claude-opus-5", "medium", "s", "t").get("fallbacks").is_some());
    }

    #[test]
    fn streaming_is_always_on() {
        assert_eq!(build_body("claude-sonnet-5", "low", "s", "t")["stream"], true);
    }
}

// ---------------------------------------------------------------------------------------
// Network side
// ---------------------------------------------------------------------------------------

use crate::config::Loaded;
use futures_util::StreamExt;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Holds the running request so it can be killed. Dropping the task drops the response
/// body, which closes the connection and stops generation — no tokens burn after dismissal.
#[derive(Default)]
pub struct InFlight(Mutex<Option<tauri::async_runtime::JoinHandle<()>>>);

impl InFlight {
    /// Only ever one critique in flight; a second hotkey press replaces the first.
    pub fn set(&self, handle: tauri::async_runtime::JoinHandle<()>) {
        let mut guard = self.0.lock().expect("in-flight lock poisoned");
        if let Some(previous) = guard.take() {
            previous.abort();
        }
        *guard = Some(handle);
    }

    /// Wired to Esc in A2.3 — dismissing the panel must stop the tokens, not just hide them.
    #[allow(dead_code)]
    pub fn abort(&self) -> bool {
        let mut guard = self.0.lock().expect("in-flight lock poisoned");
        match guard.take() {
            Some(handle) => { handle.abort(); true }
            None => false,
        }
    }
}

/// How long to wait for the TCP/TLS handshake before giving up on the endpoint.
///
/// There has to be a bound. `reqwest` defaults to none, and an endpoint that blackholes
/// packets rather than refusing them — a VPN that just dropped, a dead proxy in `base_url`,
/// captive wifi — never returns at all. The panel would sit on "reading…" forever with no
/// way to tell that from a slow model.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);

/// Per-read, and it resets on every successful read (that is why it is `read_timeout` and
/// not `timeout` — a total deadline would guillotine a long, healthy critique). The API
/// sends SSE pings while the model thinks, so a gap this long means the connection is dead,
/// not that the model is being thoughtful.
const READ_TIMEOUT: Duration = Duration::from_secs(45);

/// The host we are talking to, for error messages: `https://api.anthropic.com` →
/// `api.anthropic.com`. Naming it matters because `base_url` is configurable — when
/// somebody has pointed redpen at a local proxy, "can't reach localhost:8787" is the whole
/// diagnosis, and "can't reach api.anthropic.com" would be an actively misleading one.
fn host_of(base_url: &str) -> &str {
    let after_scheme = base_url.split("://").nth(1).unwrap_or(base_url);
    after_scheme.split('/').next().unwrap_or(after_scheme)
}

/// `#status` is one short line in a 480pt card, not a log pane.
///
/// A reqwest failure stringifies to the whole chain — "error sending request for url
/// (https://api.anthropic.com/v1/messages): client error (Connect): dns error: failed to
/// lookup address information: nodename nor servname provided" — which is accurate, wraps
/// over four lines, and tells you nothing you can act on. The chain goes to the log; the
/// card gets the one sentence that says what to do about it.
fn network_message(host: &str, e: &reqwest::Error) -> String {
    if e.is_connect() {
        format!("can’t reach {host} — check your connection")
    } else if e.is_timeout() {
        format!("{host} is not responding")
    } else {
        format!("network error talking to {host}")
    }
}

pub async fn run(app: AppHandle, loaded: Loaded, text: String) {
    let fail = |msg: String| {
        eprintln!("[redpen] {msg}");
        let _ = app.emit(EVENT_ERROR, msg);
    };

    if loaded.config.api_key.is_empty() {
        return fail("no API key in config.json".into());
    }
    if loaded.system_prompt.trim().is_empty() {
        return fail(format!(
            "system prompt is empty (system_prompt_path = {:?})",
            loaded.config.system_prompt_path
        ));
    }

    let url = format!("{}/v1/messages", loaded.config.base_url.trim_end_matches('/'));
    let host = host_of(&loaded.config.base_url);
    let body = build_body(&loaded.config.model, &loaded.config.effort, &loaded.system_prompt, &text);

    let client = match reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => return fail(format!("could not build the HTTP client: {e}")),
    };

    let mut req = client
        .post(&url)
        .header("x-api-key", &loaded.config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");
    if supports_fallbacks(&loaded.config.model) {
        req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }

    let _ = app.emit(EVENT_START, ());
    let started = std::time::Instant::now();

    let resp = match req.json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            // Full chain to the log, one actionable line to the card.
            eprintln!("[redpen] request to {url} failed after {} ms: {e:?}", started.elapsed().as_millis());
            return fail(network_message(host, &e));
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let raw = resp.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
            .unwrap_or(raw);
        return fail(format!("{status}: {detail}"));
    }

    let mut stream = resp.bytes_stream();
    let mut buf = SseBuffer::default();
    let mut stop_reason = String::from("end_turn");
    let mut first_token: Option<std::time::Duration> = None;
    let mut chars = 0usize;
    let mut splitter = ProseSplitter::default();

    while let Some(item) = stream.next().await {
        let bytes = match item {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[redpen] stream from {url} broke: {e:?}");
                // Whatever is already on screen stays there — a critique cut short is worth
                // more than a cleared card.
                return fail(if e.is_timeout() {
                    format!("{host} stopped responding mid-critique")
                } else {
                    format!("connection to {host} dropped mid-critique")
                });
            }
        };
        for frame in buf.push_bytes(&bytes) {
            match parse_frame(&frame) {
                Chunk::Text(t) => {
                    // C1.2: the trailing tag block never reaches the webview.
                    let visible = splitter.push(&t);
                    if visible.is_empty() {
                        continue;
                    }
                    if first_token.is_none() {
                        let elapsed = started.elapsed();
                        first_token = Some(elapsed);
                        // Guardrail A2 measures the first *visible* token, which is why
                        // this sits after the splitter rather than before it.
                        println!("[redpen] first token in {} ms", elapsed.as_millis());
                    }
                    chars += visible.chars().count();
                    let _ = app.emit(EVENT_DELTA, visible);
                }
                Chunk::Thinking => { let _ = app.emit(EVENT_THINKING, ()); }
                Chunk::Done(reason) => stop_reason = reason,
                Chunk::Failed(msg) => return fail(format!("stream error: {msg}")),
                Chunk::Ignore => {}
            }
        }
    }

    // Anything still held back was never a fence.
    let tail = splitter.finish();
    if !tail.is_empty() {
        chars += tail.chars().count();
        let _ = app.emit(EVENT_DELTA, tail);
    }
    // Parsed but never displayed — Epic E's journal consumes these (decision #6).
    if let Some(tags) = splitter.tags() {
        println!("[redpen] tags: [{}]", tags.join(", "));
        let _ = app.emit(EVENT_TAGS, tags);
    }

    if stop_reason == "refusal" {
        return fail("the model declined this text".into());
    }
    println!(
        "[redpen] critique done: {chars} chars, first token {} ms, total {} ms, stop={stop_reason}",
        first_token.map(|d| d.as_millis()).unwrap_or(0),
        started.elapsed().as_millis()
    );
    let _ = app.emit(EVENT_DONE, stop_reason);
}

#[cfg(test)]
mod splitter_tests {
    use super::*;

    fn feed(chunks: &[&str]) -> (String, Option<Vec<String>>) {
        let mut s = ProseSplitter::default();
        let mut out = String::new();
        for c in chunks {
            out.push_str(&s.push(c));
        }
        out.push_str(&s.finish());
        (out, s.tags())
    }

    #[test]
    fn prose_passes_through_and_tags_are_removed() {
        let (prose, tags) = feed(&["**Reads as:** off\n\n", "```json\n{\"tags\":[\"preposition\"]}\n```"]);
        assert_eq!(prose, "**Reads as:** off\n\n");
        assert_eq!(tags, Some(vec!["preposition".into()]));
    }

    #[test]
    fn a_fence_arriving_one_character_at_a_time_never_leaks() {
        // The streaming case: without hold-back the panel would flash "`", "``", "```js".
        let mut chunks: Vec<&str> = vec!["done.\n\n"];
        for c in ["`", "`", "`", "j", "s", "o", "n", "\n", "{\"tags\":[]}", "\n```"] {
            chunks.push(c);
        }
        let (prose, tags) = feed(&chunks);
        assert_eq!(prose, "done.\n\n", "no part of the fence may ever be displayed");
        assert_eq!(tags, Some(vec![]));
    }

    #[test]
    fn backticks_that_are_not_the_fence_still_render() {
        let (prose, tags) = feed(&["use `depend on` here\n"]);
        assert_eq!(prose, "use `depend on` here\n");
        assert_eq!(tags, None);
    }

    #[test]
    fn a_response_with_no_tag_block_loses_nothing() {
        let (prose, _) = feed(&["**Reads as:** near-native — nothing to flag.\n"]);
        assert_eq!(prose, "**Reads as:** near-native — nothing to flag.\n");
    }

    #[test]
    fn multibyte_prose_is_not_split_mid_character() {
        // Russian source words and curly quotes ride through the hold-back logic.
        let (prose, _) = feed(&["Russian ", "зависеть от", " carries the “from” across\n"]);
        assert_eq!(prose, "Russian зависеть от carries the “from” across\n");
    }

    #[test]
    fn a_truncated_tag_block_yields_no_tags_rather_than_garbage() {
        let (prose, tags) = feed(&["text\n", "```json\n{\"tags\":[\"art"]);
        assert_eq!(prose, "text\n");
        assert_eq!(tags, None);
    }
}

#[cfg(test)]
mod host_tests {
    use super::host_of;

    #[test]
    fn the_default_endpoint_reduces_to_its_host() {
        assert_eq!(host_of("https://api.anthropic.com"), "api.anthropic.com");
        assert_eq!(host_of("https://api.anthropic.com/"), "api.anthropic.com");
    }

    #[test]
    fn a_local_proxy_keeps_its_port() {
        // decision #7: base_url is configurable, and naming the wrong host in the error
        // would send you debugging your internet when your own proxy is down.
        assert_eq!(host_of("http://localhost:8787"), "localhost:8787");
        assert_eq!(host_of("http://127.0.0.1:8787/v1"), "127.0.0.1:8787");
    }

    #[test]
    fn a_url_with_no_scheme_still_yields_something_printable() {
        assert_eq!(host_of("api.anthropic.com"), "api.anthropic.com");
    }
}
