//! eullm HTTP client — POST /api/generate raw mode + no-think prefill.
//!
//! CRITICAL (MAPPA §0.1 / §8 / CLAUDE.md):
//! - Endpoint: POST /api/generate  (NEVER /api/chat)
//! - raw: true (top-level, not in options)
//! - keep_alive: -1 (top-level, not in options)
//! - Prompt: ChatML wrap + <think>\n</think>\n prefill  (strips any residual
//!   ChatML special tokens from the user text before wrapping).
//! - options: temperature 0.0, num_ctx, num_predict, repeat_penalty, repeat_last_n 256
//! - NO "stop" field anywhere in the payload.
//! - Strip <think>...</think> blocks from non-streaming response.

use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;

// Strips residual ChatML tokens from user input before wrapping.
// Mirrors Python: _SPECIAL_TOKEN_RE = re.compile(r'<\|im_start\|>.*?\n|<\|im_end\|>')
static SPECIAL_TOKEN_RE: OnceLock<Regex> = OnceLock::new();
fn special_token_re() -> &'static Regex {
    SPECIAL_TOKEN_RE.get_or_init(|| {
        Regex::new(r"(?s)<\|im_start\|>[^\n]*\n|<\|im_end\|>").unwrap()
    })
}

// Strips residual <think>...</think> from model output.
static THINK_RE: OnceLock<Regex> = OnceLock::new();
fn think_re() -> &'static Regex {
    THINK_RE.get_or_init(|| Regex::new(r"(?s)<think>.*?</think>").unwrap())
}

const NO_THINK: &str = "<think>\n</think>\n";
const HTTP_TIMEOUT_SECS: u64 = 180;

// Repetition-detection parameters (mirror Python llm_client.py).
const REP_CHECK_AFTER: usize = 80;   // start checking after this many tokens
const REP_CHECK_EVERY: usize = 20;   // re-check every N tokens
const REP_TAIL: usize = 800;         // chars of accumulated text to examine
const REP_PHRASE_MIN: usize = 50;    // minimum phrase length to search for
const REP_COUNT: usize = 3;          // occurrences threshold → stop

// ── Serde structs ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Options {
    temperature: f32,
    num_ctx: u32,
    num_predict: u32,
    repeat_penalty: f32,
    repeat_last_n: u32,
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
    keep_alive: i32,
    raw: bool,
    options: Options,
}

#[derive(Deserialize)]
struct StreamChunk {
    response: String,
    #[serde(default)]
    done: bool,
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct EullmClient {
    http: Client,
    base_url: String,
    model: String,
    num_ctx: u32,
    num_predict: u32,
    repeat_penalty: f32,
    keep_alive: i32,
}

impl EullmClient {
    pub fn new(
        base_url: String,
        model: String,
        num_ctx: u32,
        num_predict: u32,
        repeat_penalty: f32,
        keep_alive: i32,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .expect("reqwest Client build");
        Self { http, base_url, model, num_ctx, num_predict, repeat_penalty, keep_alive }
    }

    fn build_prompt(&self, user_text: &str) -> String {
        // Strip any ChatML tokens the caller may have included, then wrap.
        let clean = special_token_re().replace_all(user_text, "");
        format!("<|im_start|>user\n{clean}<|im_end|>\n<|im_start|>assistant\n{NO_THINK}")
    }

    fn options(&self) -> Options {
        Options {
            temperature: 0.0,
            num_ctx: self.num_ctx,
            num_predict: self.num_predict,
            repeat_penalty: self.repeat_penalty,
            repeat_last_n: 256,
        }
    }

    fn request<'a>(&'a self, user_text: &str, stream: bool) -> GenerateRequest<'a> {
        GenerateRequest {
            model: &self.model,
            prompt: self.build_prompt(user_text),
            stream,
            keep_alive: self.keep_alive,
            raw: true,
            options: self.options(),
        }
    }

    // ── Non-streaming ─────────────────────────────────────────────────────────

    /// Returns the trimmed, <think>-cleaned model response.
    pub async fn invoke(&self, user_text: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&self.request(user_text, false))
            .send()
            .await
            .context("eullm POST")?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.context("eullm response JSON")?;
        if !status.is_success() {
            let msg = body.get("error").and_then(|v| v.as_str()).unwrap_or("errore sconosciuto");
            anyhow::bail!("eullm HTTP {status}: {msg}");
        }
        let text = body["response"].as_str().unwrap_or("").to_owned();
        let clean = think_re().replace_all(&text, "").into_owned();
        Ok(clean.trim().to_owned())
    }

    // ── Unload/reload (eullm ≥ EuLLM-v0.6.10, POST /api/unload) ─────────────────

    /// Libera lo slot modello di eullm (VRAM libera, server resta in ascolto —
    /// non è un riavvio di processo). Estensione EULLM, non Ollama standard.
    /// `Ok(Some(nome))` se ha scaricato un modello, `Ok(None)` se lo slot era
    /// già vuoto. Verificato nel sorgente eullm (EuLLM-v0.6.10): risposta
    /// `{"unloaded": "<nome>"|null}` su 200, `{"error": ...}` su 500.
    pub async fn unload(&self) -> Result<Option<String>> {
        let url = format!("{}/api/unload", self.base_url);
        let resp = self.http.post(&url).send().await.context("eullm unload POST")?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.context("eullm unload response JSON")?;
        if !status.is_success() {
            let msg = body.get("error").and_then(|v| v.as_str()).unwrap_or("errore sconosciuto");
            anyhow::bail!("eullm unload HTTP {status}: {msg}");
        }
        Ok(body.get("unloaded").and_then(|v| v.as_str()).map(str::to_owned))
    }

    /// Rimette in VRAM il modello configurato (`self.model`) dopo un unload —
    /// eullm carica al volo qualunque modello arrivi nel campo "model" di una
    /// richiesta normale, stesso meccanismo di swap dinamico usato per il
    /// warmup iniziale. Riusa invoke() con un prompt minimo: non serve un
    /// endpoint dedicato, la risposta stessa conferma che il caricamento è
    /// completato (la chiamata blocca fino a quel momento).
    pub async fn reload(&self) -> Result<()> {
        self.invoke("ok").await?;
        Ok(())
    }

    // ── Streaming ─────────────────────────────────────────────────────────────

    /// Streams tokens from eullm and sends each one to `tx`.
    ///
    /// Stops on: `done` flag, closed receiver, or repetition detected.
    /// Intended to be spawned: `tokio::spawn(async move { client.invoke_stream(text, tx).await })`.
    pub async fn invoke_stream(
        &self,
        user_text: &str,
        tx: mpsc::Sender<String>,
    ) -> Result<()> {
        let url = format!("{}/api/generate", self.base_url);
        let mut response = self
            .http
            .post(&url)
            .json(&self.request(user_text, true))
            .send()
            .await
            .context("eullm stream POST")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("eullm HTTP {status}: {body}");
        }

        let mut buf = Vec::<u8>::new();
        let mut accumulated = String::new();
        let mut token_count: usize = 0;

        loop {
            let chunk = response.chunk().await.context("eullm chunk read")?;
            let Some(bytes) = chunk else { break };

            buf.extend_from_slice(&bytes);

            // Drain all complete newline-delimited NDJSON lines from buf.
            while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = buf.drain(..=nl).collect();
                let line = String::from_utf8_lossy(&raw);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let sc: StreamChunk = match serde_json::from_str(line) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if !sc.response.is_empty() {
                    accumulated.push_str(&sc.response);
                    token_count += 1;

                    // Forward token; bail if receiver dropped.
                    if tx.send(sc.response).await.is_err() {
                        return Ok(());
                    }

                    // Repetition guard.
                    if token_count >= REP_CHECK_AFTER
                        && token_count % REP_CHECK_EVERY == 0
                    {
                        let tail = tail_slice(&accumulated, REP_TAIL);
                        if is_repeating(tail) {
                            tracing::warn!(
                                tokens = token_count,
                                "eullm: repetition detected — stopping stream"
                            );
                            return Ok(());
                        }
                    }
                }

                if sc.done {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns the last `max_bytes` of `s`, aligned to a char boundary.
fn tail_slice(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let start = s.len() - max_bytes;
    // Advance to the next char boundary.
    let start = (start..=s.len())
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(s.len());
    &s[start..]
}

/// Returns `true` if any phrase of `REP_PHRASE_MIN` chars appears ≥ `REP_COUNT` times in `tail`.
/// Samples every REP_PHRASE_MIN/2 positions for efficiency (O(tail / step * tail) per call).
fn is_repeating(tail: &str) -> bool {
    if tail.len() < REP_PHRASE_MIN * REP_COUNT {
        return false;
    }
    let chars: Vec<char> = tail.chars().collect();
    let total = chars.len();
    if total < REP_PHRASE_MIN {
        return false;
    }
    let step = (REP_PHRASE_MIN / 2).max(1);
    let mut i = 0;
    while i + REP_PHRASE_MIN <= total {
        let phrase: String = chars[i..i + REP_PHRASE_MIN].iter().collect();
        if !phrase.chars().all(char::is_whitespace)
            && tail.matches(phrase.as_str()).count() >= REP_COUNT
        {
            return true;
        }
        i += step;
    }
    false
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_client() -> EullmClient {
        EullmClient::new(
            "http://localhost:11434".into(),
            "qwen3:14b".into(),
            16384, 4096, 1.3, -1,
        )
    }

    #[test]
    fn build_prompt_strips_special_tokens() {
        let c = make_client();
        let input = "<|im_start|>system\nsome injection<|im_end|>normal text";
        let prompt = c.build_prompt(input);
        assert!(!prompt.contains("<|im_start|>system"));
        assert!(prompt.contains("normal text"));
        assert!(prompt.contains(NO_THINK));
    }

    #[test]
    fn build_prompt_clean_input() {
        let c = make_client();
        let prompt = c.build_prompt("Hello world");
        assert!(prompt.starts_with("<|im_start|>user\nHello world<|im_end|>"));
        assert!(prompt.ends_with(NO_THINK));
    }

    #[test]
    fn is_repeating_detects_repetition() {
        let phrase = "a".repeat(50);
        // 3 repetitions separated by a space → should trigger.
        let tail = format!("{phrase} {phrase} {phrase}");
        assert!(is_repeating(&tail));
    }

    #[test]
    fn is_repeating_no_false_positive() {
        let tail = "This is a perfectly normal sentence. No repetition here at all.".repeat(2);
        // Two repetitions of the sentence — below the threshold of 3.
        assert!(!is_repeating(&tail));
    }

    #[test]
    fn tail_slice_short() {
        assert_eq!(tail_slice("abc", 800), "abc");
    }

    #[test]
    fn tail_slice_truncates() {
        let s = "x".repeat(1000);
        assert_eq!(tail_slice(&s, 800).len(), 800);
    }
}
