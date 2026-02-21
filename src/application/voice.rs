// Voice message processing: transcribe audio and route to agent.

use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
use crate::domain::voice::{TranscriptionError, VoiceTranscriber};

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
/// user-friendly error message on failure.
pub async fn process_voice_message(
    whisper: &dyn VoiceTranscriber,
    audio_bytes: Vec<u8>,
    file_name: &str,
    agent: &dyn AgentLoop,
) -> Result<VoiceProcessingResult, String> {
    let transcription = whisper
        .transcribe_bytes(audio_bytes, file_name)
        .await
        .map_err(|e| match &e {
            TranscriptionError::NotConfigured(_) => {
                "voice transcription is not configured".to_string()
            }
            _ => format!("transcription failed: {}", e),
        })?;

    let mut messages = vec![Message {
        role: Role::User,
        content: transcription.text.clone(),
        tool_calls: vec![],
        tool_call_id: None,
    }];

    let result = agent
        .process(&mut messages)
        .await
        .map_err(|e| format!("agent processing failed: {}", e))?;

    Ok(VoiceProcessingResult {
        transcription: transcription.text,
        agent_response: result.response,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{AgentInfo, AgentResult};
    use crate::domain::error::DomainError;
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
    async fn test_no_api_key_returns_error() {
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
        let err = result.unwrap_err();
        assert!(
            err.contains("voice transcription is not configured"),
            "expected 'voice transcription is not configured', got: {}",
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
    async fn test_service_error() {
        let transcriber = StubTranscriber {
            result: Err(TranscriptionError::ServiceError(
                "API error (500): error".into(),
            )),
        };
        let agent = StubAgent {
            response: "ok".into(),
        };
        let result =
            process_voice_message(&transcriber, b"audio".to_vec(), "test.ogg", &agent).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("transcription failed"));
    }
}
