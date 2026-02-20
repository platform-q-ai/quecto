// Groq Whisper: voice-to-text transcription for Telegram voice messages.

use reqwest::Client;
use serde::Deserialize;
use std::path::Path;

/// Result of a transcription request.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
}

/// Error from the Groq Whisper transcription client.
#[derive(Debug)]
pub enum TranscriptionError {
    /// No API key configured.
    NoApiKey(String),
    /// HTTP or network error.
    Http(String),
    /// API returned a non-success status.
    ApiError(u16, String),
    /// Failed to read audio file.
    FileError(String),
    /// Failed to parse response.
    ParseError(String),
}

impl std::fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptionError::NoApiKey(msg) => write!(f, "{}", msg),
            TranscriptionError::Http(msg) => write!(f, "HTTP error: {}", msg),
            TranscriptionError::ApiError(status, msg) => {
                write!(f, "API error ({}): {}", status, msg)
            }
            TranscriptionError::FileError(msg) => write!(f, "file error: {}", msg),
            TranscriptionError::ParseError(msg) => write!(f, "parse error: {}", msg),
        }
    }
}

impl std::error::Error for TranscriptionError {}

/// Response from Groq Whisper API.
#[derive(Debug, Deserialize)]
struct WhisperResponse {
    text: String,
}

/// Client for the Groq Whisper speech-to-text API.
#[derive(Debug, Clone)]
pub struct GroqWhisperClient {
    api_key: String,
    api_base: String,
    client: Client,
    model: String,
}

impl GroqWhisperClient {
    /// Create a new client with the given API key.
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            api_base: "https://api.groq.com".to_string(),
            client: Client::new(),
            model: "whisper-large-v3-turbo".to_string(),
        }
    }

    /// Create a new client with a custom API base URL (for testing).
    pub fn with_base_url(api_key: &str, api_base: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            api_base: api_base.to_string(),
            client: Client::new(),
            model: "whisper-large-v3-turbo".to_string(),
        }
    }

    /// Transcribe an audio file at the given path.
    pub async fn transcribe(
        &self,
        audio_path: &Path,
    ) -> Result<TranscriptionResult, TranscriptionError> {
        if self.api_key.is_empty() {
            return Err(TranscriptionError::NoApiKey(
                "api key not configured".to_string(),
            ));
        }

        let file_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| TranscriptionError::FileError(e.to_string()))?;

        let file_name = audio_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str("audio/ogg")
            .map_err(|e| TranscriptionError::Http(e.to_string()))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .part("file", file_part);

        let url = format!("{}/openai/v1/audio/transcriptions", self.api_base);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| TranscriptionError::Http(e.to_string()))?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(TranscriptionError::ApiError(status, body));
        }

        let whisper_resp: WhisperResponse = response
            .json()
            .await
            .map_err(|e| TranscriptionError::ParseError(e.to_string()))?;

        Ok(TranscriptionResult {
            text: whisper_resp.text,
        })
    }

    /// Transcribe raw audio bytes directly (useful when audio is already in memory).
    pub async fn transcribe_bytes(
        &self,
        audio_bytes: Vec<u8>,
        file_name: &str,
    ) -> Result<TranscriptionResult, TranscriptionError> {
        if self.api_key.is_empty() {
            return Err(TranscriptionError::NoApiKey(
                "api key not configured".to_string(),
            ));
        }

        let file_part = reqwest::multipart::Part::bytes(audio_bytes)
            .file_name(file_name.to_string())
            .mime_str("audio/ogg")
            .map_err(|e| TranscriptionError::Http(e.to_string()))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .part("file", file_part);

        let url = format!("{}/openai/v1/audio/transcriptions", self.api_base);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| TranscriptionError::Http(e.to_string()))?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(TranscriptionError::ApiError(status, body));
        }

        let whisper_resp: WhisperResponse = response
            .json()
            .await
            .map_err(|e| TranscriptionError::ParseError(e.to_string()))?;

        Ok(TranscriptionResult {
            text: whisper_resp.text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = GroqWhisperClient::new("gsk-test");
        assert_eq!(client.api_key, "gsk-test");
        assert_eq!(client.api_base, "https://api.groq.com");
        assert_eq!(client.model, "whisper-large-v3-turbo");
    }

    #[test]
    fn test_client_with_custom_base() {
        let client = GroqWhisperClient::with_base_url("gsk-test", "http://localhost:9999");
        assert_eq!(client.api_base, "http://localhost:9999");
    }

    #[tokio::test]
    async fn test_no_api_key_returns_error() {
        let client = GroqWhisperClient::new("");
        let result = client.transcribe(Path::new("/tmp/test.ogg")).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("api key not configured"),
            "expected 'api key not configured', got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_no_api_key_bytes_returns_error() {
        let client = GroqWhisperClient::new("");
        let result = client.transcribe_bytes(vec![1, 2, 3], "test.ogg").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("api key not configured")
        );
    }

    #[tokio::test]
    async fn test_file_not_found_error() {
        let client = GroqWhisperClient::with_base_url("gsk-test", "http://localhost:1");
        let result = client.transcribe(Path::new("/nonexistent/audio.ogg")).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TranscriptionError::FileError(_) => {}
            other => panic!("expected FileError, got: {:?}", other),
        }
    }

    #[test]
    fn test_error_display() {
        let err = TranscriptionError::NoApiKey("api key not configured".to_string());
        assert_eq!(err.to_string(), "api key not configured");

        let err = TranscriptionError::ApiError(429, "rate limited".to_string());
        assert_eq!(err.to_string(), "API error (429): rate limited");

        let err = TranscriptionError::Http("connection refused".to_string());
        assert_eq!(err.to_string(), "HTTP error: connection refused");
    }
}
