//! Shared OpenAI-compatible chat-completions client.
//!
//! DeepSeek, Zen and Ollama all speak this dialect; each concrete channel is
//! a thin configuration of this client (base URL, auth, model naming).

use crate::channels::sse::SseParser;
use crate::types::{ChannelError, GenChunk, GenRequest};
use futures::StreamExt;
use futures::stream::BoxStream;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

/// Wire format of one streamed chat-completion chunk (the fields we use).
#[derive(Debug, Deserialize)]
struct WireChunk {
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
struct WireChoice {
    #[serde(default)]
    delta: WireDelta,
}

#[derive(Debug, Deserialize, Default)]
struct WireDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

/// Map one SSE `data:` payload to chunks. Pure — unit-testable without HTTP.
pub fn payload_to_chunks(payload: &str) -> Vec<GenChunk> {
    if payload.trim() == "[DONE]" {
        return vec![GenChunk::Done];
    }
    let Ok(wire) = serde_json::from_str::<WireChunk>(payload) else {
        return Vec::new(); // tolerate unknown keep-alive payloads
    };
    let mut out = Vec::new();
    if let Some(usage) = wire.usage {
        out.push(GenChunk::Usage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        });
    }
    for choice in wire.choices {
        if let Some(text) = choice.delta.content
            && !text.is_empty()
        {
            out.push(GenChunk::Text { text });
        }
    }
    out
}

/// Map an HTTP error status to a channel error (§11.E).
pub fn status_to_error(status: u16, body: &str, retry_after: Option<u64>) -> ChannelError {
    match status {
        401 | 403 => ChannelError::BadKey,
        402 => ChannelError::InsufficientBalance,
        429 => ChannelError::RateLimited {
            retry_after_secs: retry_after.unwrap_or(8),
        },
        s => ChannelError::Backend {
            status: s,
            message: truncate(body, 300),
        },
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Configuration for one OpenAI-compatible endpoint.
pub struct OpenAiCompatClient {
    pub base_url: String,
    pub api_key: Option<SecretString>,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl OpenAiCompatClient {
    pub fn new(base_url: impl Into<String>, api_key: Option<SecretString>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(300),
        }
    }

    fn http(&self) -> Result<reqwest::Client, ChannelError> {
        reqwest::Client::builder()
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .build()
            .map_err(|e| ChannelError::Network(e.to_string()))
    }

    /// GET `{base}/models` — used by probes ([测试连接], §4.7).
    pub async fn list_models(&self) -> Result<Vec<String>, ChannelError> {
        let mut req = self.http()?.get(format!("{}/models", self.base_url));
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key.expose_secret());
        }
        let resp = req.send().await.map_err(map_reqwest_err)?;
        let status = resp.status().as_u16();
        if status != 200 {
            let body = resp.text().await.unwrap_or_default();
            return Err(status_to_error(status, &body, None));
        }
        #[derive(Deserialize)]
        struct ModelList {
            #[serde(default)]
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }
        let list: ModelList = resp
            .json()
            .await
            .map_err(|e| ChannelError::Network(e.to_string()))?;
        Ok(list.data.into_iter().map(|m| m.id).collect())
    }

    /// POST `{base}/chat/completions` with `stream: true`.
    pub async fn complete_stream(
        &self,
        req: GenRequest,
    ) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError> {
        let mut body = json!({
            "model": req.model,
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": [
                {"role": "system", "content": req.system},
                {"role": "user", "content": req.user},
            ],
        });
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            body["max_tokens"] = json!(m);
        }

        let mut http_req = self
            .http()?
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if let Some(key) = &self.api_key {
            http_req = http_req.bearer_auth(key.expose_secret());
        }
        let resp = http_req.send().await.map_err(map_reqwest_err)?;
        let status = resp.status().as_u16();
        if status != 200 {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            let text = resp.text().await.unwrap_or_default();
            return Err(status_to_error(status, &text, retry_after));
        }

        let byte_stream = resp.bytes_stream();
        let stream = byte_stream
            .map(|item| item.map_err(|e| ChannelError::Network(e.to_string())))
            .scan(SseParser::new(), |parser, item| {
                let out: Vec<Result<GenChunk, ChannelError>> = match item {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        parser
                            .push(&text)
                            .iter()
                            .flat_map(|payload| payload_to_chunks(payload))
                            .map(Ok)
                            .collect()
                    }
                    Err(e) => vec![Err(e)],
                };
                futures::future::ready(Some(futures::stream::iter(out)))
            })
            .flatten();
        Ok(stream.boxed())
    }
}

fn map_reqwest_err(e: reqwest::Error) -> ChannelError {
    if e.is_timeout() {
        ChannelError::Timeout
    } else {
        ChannelError::Network(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_parses() {
        let payload = r#"{"choices":[{"delta":{"content":"Hel"}}]}"#;
        assert_eq!(
            payload_to_chunks(payload),
            vec![GenChunk::Text { text: "Hel".into() }]
        );
    }

    #[test]
    fn done_marker() {
        assert_eq!(payload_to_chunks("[DONE]"), vec![GenChunk::Done]);
    }

    #[test]
    fn usage_chunk_parses() {
        let payload = r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":42}}"#;
        assert_eq!(
            payload_to_chunks(payload),
            vec![GenChunk::Usage {
                prompt_tokens: 100,
                completion_tokens: 42
            }]
        );
    }

    #[test]
    fn garbage_is_tolerated() {
        assert!(payload_to_chunks("not json").is_empty());
    }

    #[test]
    fn status_mapping() {
        assert!(matches!(
            status_to_error(401, "", None),
            ChannelError::BadKey
        ));
        assert!(matches!(
            status_to_error(402, "", None),
            ChannelError::InsufficientBalance
        ));
        assert!(matches!(
            status_to_error(429, "", Some(15)),
            ChannelError::RateLimited {
                retry_after_secs: 15
            }
        ));
        assert!(matches!(
            status_to_error(500, "boom", None),
            ChannelError::Backend { status: 500, .. }
        ));
    }
}
