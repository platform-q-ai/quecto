// Voice transcription: domain types and traits.

use std::future::Future;
use std::pin::Pin;

/// Result of a transcription request.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
}

/// Error from a voice transcription service.
#[derive(Debug)]
pub enum TranscriptionError {
    /// No API key configured.
    NotConfigured(String),
    /// Service error (HTTP, parsing, etc.).
    ServiceError(String),
}

impl std::fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptionError::NotConfigured(msg) => write!(f, "{}", msg),
            TranscriptionError::ServiceError(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for TranscriptionError {}

/// Trait for voice-to-text transcription services.
pub trait VoiceTranscriber: Send + Sync {
    /// Transcribe raw audio bytes into text.
    fn transcribe_bytes(
        &self,
        audio_bytes: Vec<u8>,
        file_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<TranscriptionResult, TranscriptionError>> + Send + '_>>;
}
