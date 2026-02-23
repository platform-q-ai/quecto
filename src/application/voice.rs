// Voice message processing: transcribe audio and route to agent.

use crate::domain::agent::AgentLoop;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::voice::{TranscriptionError, VoiceTranscriber};

/// Maximum audio size in bytes (25 MB — Whisper API limit).
const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;

/// Result of processing a voice message.
#[derive(Debug)]
pub struct VoiceProcessingResult {
    /// The transcribed text from the audio.
    pub transcription: String,
    /// The agent's response to the transcribed text.
    pub agent_response: String,
}

/// Process a voice message: transcribe audio bytes and route to the agent.
///
/// Returns the transcription and agent response on success, or a
/// `DomainError` on failure. Error messages are sanitized for user display;
/// internal details are not exposed.
pub async fn process_voice_message(
    whisper: &dyn VoiceTranscriber,
    audio_bytes: Vec<u8>,
    file_name: &str,
    agent: &dyn AgentLoop,
) -> Result<VoiceProcessingResult, DomainError> {
    if audio_bytes.len() > MAX_AUDIO_BYTES {
        return Err(DomainError::Tool(
            "audio file too large (max 25 MB)".to_string(),
        ));
    }

    let transcription = whisper
        .transcribe_bytes(audio_bytes, file_name)
        .await
        .map_err(|e| match e {
            TranscriptionError::NotConfigured(_) => {
                DomainError::Config("voice transcription is not configured".into())
            }
            TranscriptionError::ServiceError(_) => {
                DomainError::Provider("voice transcription failed".into())
            }
        })?;

    let mut messages = vec![Message::user(transcription.text.clone())];

    let result = agent.process(&mut messages).await?;

    Ok(VoiceProcessingResult {
        transcription: transcription.text,
        agent_response: result.response,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{AgentInfo, AgentResult};
    use crate::domain::voice::TranscriptionResult;
    use std::future::Future;
    use std::pin::Pin;

    struct StubAgent {
        response: String,
    }

    impl AgentLoop for StubAgent {
        fn process<'a>(
            &'a self,
            _messages: &'a mut Vec<Message>,
        ) -> Pin<Box<dyn Future<Output = Result<AgentResult, DomainError>> + Send + 'a>> {
            let resp = self.response.clone();
            Box::pin(async move { Ok(AgentResult::text(resp)) })
        }

        fn info(&self) -> AgentInfo {
            AgentInfo {
                tool_count: 0,
                skill_count: 0,
            }
        }
    }

    struct StubTranscriber {
        result: Result<TranscriptionResult, TranscriptionError>,
    }

    impl VoiceTranscriber for StubTranscriber {
        fn transcribe_bytes(
            &self,
            _audio_bytes: Vec<u8>,
            _file_name: &str,
        ) -> Pin<
            Box<dyn Future<Output = Result<TranscriptionResult, TranscriptionError>> + Send + '_>,
        > {
            let text = self
                .result
                .as_ref()
                .map(|r| r.text.clone())
                .map_err(|e| match e {
                    TranscriptionError::NotConfigured(msg) => {
                        TranscriptionError::NotConfigured(msg.clone())
                    }
                    TranscriptionError::ServiceError(msg) => {
                        TranscriptionError::ServiceError(msg.clone())
                    }
                });
            Box::pin(async move { text.map(|t| TranscriptionResult { text: t }) })
        }
    }

    #[tokio::test]
    async fn test_no_api_key_returns_config_error() {
        let transcriber = StubTranscriber {
            result: Err(TranscriptionError::NotConfigured(
                "api key not configured".into(),
            )),
        };
        let agent = StubAgent {
            response: "ok".into(),
        };
        let result = process_voice_message(&transcriber, vec![1, 2], "test.ogg", &agent).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("voice transcription is not configured"),
            "expected config error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_success() {
        let transcriber = StubTranscriber {
            result: Ok(TranscriptionResult {
                text: "hello".into(),
            }),
        };
        let agent = StubAgent {
            response: "I heard you".into(),
        };
        let result = process_voice_message(&transcriber, b"audio".to_vec(), "test.ogg", &agent)
            .await
            .unwrap();
        assert_eq!(result.transcription, "hello");
        assert_eq!(result.agent_response, "I heard you");
    }

    #[tokio::test]
    async fn test_service_error_is_sanitized() {
        let transcriber = StubTranscriber {
            result: Err(TranscriptionError::ServiceError(
                "API error (500): internal stack trace here".into(),
            )),
        };
        let agent = StubAgent {
            response: "ok".into(),
        };
        let result =
            process_voice_message(&transcriber, b"audio".to_vec(), "test.ogg", &agent).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Error should be sanitized — no internal details leaked
        assert!(
            err.contains("voice transcription failed"),
            "expected sanitized error, got: {}",
            err
        );
        assert!(
            !err.contains("stack trace"),
            "internal details leaked: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_audio_too_large() {
        let transcriber = StubTranscriber {
            result: Ok(TranscriptionResult {
                text: "hello".into(),
            }),
        };
        let agent = StubAgent {
            response: "ok".into(),
        };
        let large_audio = vec![0u8; MAX_AUDIO_BYTES + 1];
        let result = process_voice_message(&transcriber, large_audio, "test.ogg", &agent).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }
}
