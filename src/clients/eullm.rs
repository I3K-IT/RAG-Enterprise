//! eullm HTTP client — POST /api/generate raw mode + no-think prefill.
//!
//! CRITICAL (MAPPA §0.1 / §8):
//! - Endpoint: POST /api/generate  (NEVER /api/chat)
//! - raw: true
//! - Prompt format: <|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n<think>\n</think>\n
//! - Payload: model, prompt, stream, keep_alive:-1, raw:true,
//!            options:{temperature:0.0, num_ctx:16384, num_predict:4096,
//!                    repeat_penalty:1.3, repeat_last_n:256}
//! - Strip residual <think>...</think> from response.
//! - RAG prompts also start with the textual directive "/no_think".

use anyhow::Result;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static THINK_RE: OnceLock<Regex> = OnceLock::new();

fn think_re() -> &'static Regex {
    THINK_RE.get_or_init(|| Regex::new(r"(?s)<think>.*?</think>").unwrap())
}

const NO_THINK: &str = "<think>\n</think>\n";

#[derive(Debug, Serialize)]
struct GenerateOptions {
    temperature: f32,
    num_ctx: u32,
    num_predict: u32,
    repeat_penalty: f32,
    repeat_last_n: u32,
}

#[derive(Debug, Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
    keep_alive: i32,
    raw: bool,
    options: GenerateOptions,
}

#[derive(Debug, Deserialize)]
struct GenerateResponse {
    response: String,
}

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
    pub fn new(base_url: String, model: String, num_ctx: u32, num_predict: u32, repeat_penalty: f32, keep_alive: i32) -> Self {
        Self {
            http: Client::new(),
            base_url,
            model,
            num_ctx,
            num_predict,
            repeat_penalty,
            keep_alive,
        }
    }

    fn build_prompt(&self, user_text: &str) -> String {
        format!(
            "<|im_start|>user\n{user_text}<|im_end|>\n<|im_start|>assistant\n{NO_THINK}"
        )
    }

    pub async fn invoke(&self, user_text: &str) -> Result<String> {
        let prompt = self.build_prompt(user_text);
        let req = GenerateRequest {
            model: &self.model,
            prompt,
            stream: false,
            keep_alive: self.keep_alive,
            raw: true,
            options: GenerateOptions {
                temperature: 0.0,
                num_ctx: self.num_ctx,
                num_predict: self.num_predict,
                repeat_penalty: self.repeat_penalty,
                repeat_last_n: 256,
            },
        };
        let url = format!("{}/api/generate", self.base_url);
        let resp: GenerateResponse = self.http.post(&url).json(&req).send().await?.json().await?;
        let clean = think_re().replace_all(&resp.response, "").into_owned();
        Ok(clean.trim().to_owned())
    }

    pub async fn invoke_json(&self, user_text: &str) -> Result<serde_json::Value> {
        // TODO Fase 1: add format:"json" to payload when json_mode=true
        let text = self.invoke(user_text).await?;
        Ok(serde_json::from_str(&text)?)
    }
}
