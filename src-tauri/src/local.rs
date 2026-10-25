//! A local model provider: Ollama running on this computer.
//!
//! It takes the same system prompt and JSON Schema as the Claude provider and
//! returns the same `RawReply`, so `ai::validate` (quote checks, page fixes,
//! date checks) treats both identically. Nothing leaves the computer: the
//! request goes to 127.0.0.1 only.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::ai::{RawReply, Usage};
use crate::error::AppError;

pub const DEFAULT_LOCAL_MODEL: &str = "qwen2.5:3b";
const BASE: &str = "http://127.0.0.1:11434";
const TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct OllamaClient {
    pub model: String,
}

impl OllamaClient {
    pub fn from_env() -> Self {
        let model = std::env::var("MYFILE_LOCAL_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_LOCAL_MODEL.to_string());
        Self { model }
    }

    /// Whether Ollama is running and has this model installed.
    pub fn status(&self) -> Result<(), AppError> {
        let tags: Value = ureq::get(&format!("{BASE}/api/tags"))
            .timeout(Duration::from_secs(3))
            .call()
            .map_err(|_| {
                AppError::new(
                    "local_unavailable",
                    "The local model needs Ollama running on this computer (https://ollama.com). Start Ollama and try again.",
                )
            })?
            .into_json()
            .map_err(|_| AppError::new("local_unavailable", "Ollama answered in an unexpected way."))?;
        let installed = tags["models"]
            .as_array()
            .is_some_and(|ms| ms.iter().any(|m| m["name"].as_str().is_some_and(|n| n == self.model || n == format!("{}:latest", self.model))));
        if installed {
            Ok(())
        } else {
            Err(AppError::new(
                "local_model_missing",
                format!("Ollama is running but the model “{}” isn't installed. Run: ollama pull {}", self.model, self.model),
            ))
        }
    }

    pub async fn run_with(
        &self,
        system_prompt: &str,
        schema: &str,
        prompt: String,
        cancel: oneshot::Receiver<()>,
    ) -> Result<RawReply, AppError> {
        let schema: Value = serde_json::from_str(schema).expect("schemas are valid JSON");
        let body = json!({
            "model": self.model,
            "stream": false,
            "format": schema,
            "options": { "temperature": 0 },
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": prompt },
            ],
        });
        let model = self.model.clone();
        let request = tauri::async_runtime::spawn_blocking(move || {
            ureq::post(&format!("{BASE}/api/chat")).timeout(TIMEOUT).send_json(body)
        });
        // If cancelled, stop waiting. Ollama finishes the request on its own,
        // but nothing is shown or kept.
        let response = tokio::select! {
            r = request => r.map_err(|_| AppError::new("internal", "The local request stopped unexpectedly."))?,
            Ok(()) = cancel => return Err(AppError::new("ai_cancelled", "Cancelled. Nothing was saved.")),
        };
        let v: Value = match response {
            Ok(resp) => resp
                .into_json()
                .map_err(|_| AppError::new("ai_invalid", "The local model's reply wasn't valid JSON."))?,
            Err(ureq::Error::Status(code, resp)) => {
                let detail = resp.into_string().unwrap_or_default();
                return Err(AppError::new("ai_failed", format!("Ollama returned an error ({code}): {}", detail.chars().take(200).collect::<String>())));
            }
            Err(e) => return Err(AppError::new("local_unavailable", format!("Couldn't reach Ollama ({e}). Is it running?"))),
        };
        parse_chat(&v, &model)
    }
}

/// Turns Ollama's /api/chat response into the same shape the Claude provider returns.
fn parse_chat(v: &Value, model: &str) -> Result<RawReply, AppError> {
    let content = v["message"]["content"].as_str().unwrap_or("").trim();
    if content.is_empty() {
        return Err(AppError::new("ai_empty", "The local model returned an empty answer."));
    }
    let json: Value = serde_json::from_str(content)
        .map_err(|_| AppError::new("ai_invalid", "The local model replied, but not in the expected format."))?;
    let usage = Usage {
        cost_usd: 0.0,
        input_tokens: v["prompt_eval_count"].as_u64().unwrap_or(0),
        output_tokens: v["eval_count"].as_u64().unwrap_or(0),
        api_ms: v["total_duration"].as_u64().unwrap_or(0) / 1_000_000,
    };
    Ok(RawReply { json, model: format!("{model} (local)"), usage })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ollama_chat_responses() {
        let v = json!({"message":{"role":"assistant","content":"{\"answer\":\"a\",\"found\":true,\"items\":[]}"},
                       "prompt_eval_count": 1200, "eval_count": 80, "total_duration": 4_500_000_000u64});
        let r = parse_chat(&v, "qwen2.5:3b").unwrap();
        assert_eq!(r.json["answer"], "a");
        assert_eq!(r.model, "qwen2.5:3b (local)");
        assert_eq!((r.usage.input_tokens, r.usage.output_tokens, r.usage.api_ms), (1200, 80, 4500));
        assert_eq!(r.usage.cost_usd, 0.0);
        let bad = json!({"message":{"content":"not json"}});
        assert_eq!(parse_chat(&bad, "m").err().unwrap().code, "ai_invalid");
        let empty = json!({"message":{"content":"  "}});
        assert_eq!(parse_chat(&empty, "m").err().unwrap().code, "ai_empty");
    }
}
