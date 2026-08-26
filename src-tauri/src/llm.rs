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
    let body = build_body(&loaded.config.model, &loaded.config.effort, &loaded.system_prompt, &text);

    let mut req = reqwest::Client::new()
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
        Err(e) => return fail(format!("request failed: {e}")),
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

    while let Some(item) = stream.next().await {
        let bytes = match item {
            Ok(b) => b,
            Err(e) => return fail(format!("stream broke: {e}")),
        };
        for frame in buf.push_bytes(&bytes) {
            match parse_frame(&frame) {
                Chunk::Text(t) => {
                    if first_token.is_none() {
                        let elapsed = started.elapsed();
                        first_token = Some(elapsed);
                        // Guardrail A2: first visible token < 1.5 s p50.
                        println!("[redpen] first token in {} ms", elapsed.as_millis());
                    }
                    chars += t.chars().count();
                    let _ = app.emit(EVENT_DELTA, t);
                }
                Chunk::Thinking => { let _ = app.emit(EVENT_THINKING, ()); }
                Chunk::Done(reason) => stop_reason = reason,
                Chunk::Failed(msg) => return fail(format!("stream error: {msg}")),
                Chunk::Ignore => {}
            }
        }
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
