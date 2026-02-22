// Telegram polling, update dispatch, and voice message handling.

use tokio::sync::mpsc;

use crate::infrastructure::bus::InboundMessage;
use crate::infrastructure::channels::telegram::{TelegramChannel, TelegramUpdate};
use crate::infrastructure::config::Config;
use crate::infrastructure::voice::groq_whisper::GroqWhisperClient;

use super::{Gateway, handle_bot_command};

pub(super) const MAX_VOICE_BYTES: usize = 10 * 1024 * 1024;
pub(super) const ALLOW_INSECURE_VOICE_API_BASE_ENV: &str =
    "QUECTO_ALLOW_INSECURE_TELEGRAM_API_BASE";

pub(super) struct UpdateDispatchContext<'a> {
    pub(super) config: &'a Config,
    pub(super) whisper: Option<GroqWhisperClient>,
}

pub(super) struct VoicePayload {
    pub(super) chat_id: String,
    pub(super) file_id: String,
    pub(super) file_size: Option<u64>,
}

impl Gateway {
    /// Telegram long-polling task.
    pub(super) async fn run_telegram_polling(
        telegram: TelegramChannel,
        inbound_tx: mpsc::Sender<InboundMessage>,
        config: Config,
    ) {
        if !telegram.is_enabled() {
            tracing::info!("Telegram disabled, polling not started");
            std::future::pending::<()>().await;
            return;
        }

        tracing::info!("Telegram polling started");
        let mut offset: i64 = 0;

        let allow_insecure = matches!(
            std::env::var(ALLOW_INSECURE_VOICE_API_BASE_ENV)
                .ok()
                .as_deref(),
            Some("1") | Some("true") | Some("TRUE") | Some("True")
        );
        let dispatch_ctx = UpdateDispatchContext {
            whisper: Self::build_whisper_client(&config.voice, allow_insecure),
            config: &config,
        };

        loop {
            let poll_result = Self::poll_once(&telegram, &inbound_tx, offset, &dispatch_ctx).await;
            match poll_result {
                Ok(new_offset) => offset = new_offset,
                Err(()) => return,
            }
        }
    }

    /// Execute one poll cycle. Returns updated offset, or Err if channel closed.
    pub(super) async fn poll_once(
        telegram: &TelegramChannel,
        inbound_tx: &mpsc::Sender<InboundMessage>,
        mut offset: i64,
        ctx: &UpdateDispatchContext<'_>,
    ) -> Result<i64, ()> {
        match telegram.get_updates(offset, 30).await {
            Ok(updates) => {
                for update in updates {
                    offset = update.update_id + 1;
                    Self::dispatch_update(telegram, &update, inbound_tx, ctx).await?;
                }
                Ok(offset)
            }
            Err(e) => {
                tracing::error!(error = %e, "Telegram getUpdates failed, retrying in 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok(offset)
            }
        }
    }

    /// Process a single Telegram update and dispatch it to the inbound channel.
    ///
    /// Known bot commands (`/start`, `/help`, `/status`) are handled directly
    /// and responded to via `telegram.send_message()` without going through
    /// the agent loop. Voice messages are transcribed via Groq Whisper before
    /// routing. Unknown messages (including unknown commands) are forwarded to
    /// the inbound channel for agent processing.
    ///
    /// Returns Err(()) if the inbound channel is closed.
    pub(super) async fn dispatch_update(
        telegram: &TelegramChannel,
        update: &TelegramUpdate,
        inbound_tx: &mpsc::Sender<InboundMessage>,
        ctx: &UpdateDispatchContext<'_>,
    ) -> Result<(), ()> {
        // Try text message first.
        if let Some(msg) = TelegramChannel::parse_update(update) {
            if !telegram.is_user_allowed(&msg.sender_id) {
                tracing::warn!(sender_id = msg.sender_id, "unauthorized Telegram user");
                return Ok(());
            }

            // Check for bot commands before routing to agent.
            if let Some(response) = handle_bot_command(&msg.text, ctx.config) {
                if let Err(e) = telegram.send_message(&msg.chat_id, &response).await {
                    tracing::error!(error = %e, "failed to send bot command response");
                }
                return Ok(());
            }

            let inbound = InboundMessage {
                source: format!("telegram:{}", msg.chat_id),
                sender_id: msg.sender_id,
                text: msg.text,
            };
            return inbound_tx.send(inbound).await.map_err(|_| {
                tracing::error!("inbound channel closed");
            });
        }

        // Try voice message.
        if let Some((sender_id, chat_id, file_id, file_size)) =
            TelegramChannel::parse_voice_update(update)
        {
            if !telegram.is_user_allowed(&sender_id) {
                tracing::warn!(sender_id = sender_id, "unauthorized Telegram user");
                return Ok(());
            }

            let telegram_cloned = telegram.clone();
            let inbound_tx_cloned = inbound_tx.clone();
            let whisper = ctx.whisper.clone();

            tokio::spawn(async move {
                let payload = VoicePayload {
                    chat_id: chat_id.clone(),
                    file_id,
                    file_size,
                };
                let text = Self::handle_voice_message(&telegram_cloned, &payload, whisper).await;

                let Some(transcribed_text) = text else {
                    return;
                };

                let inbound = InboundMessage {
                    source: format!("telegram:{}", chat_id),
                    sender_id,
                    text: transcribed_text,
                };
                if inbound_tx_cloned.send(inbound).await.is_err() {
                    tracing::error!("inbound channel closed");
                }
            });

            return Ok(());
        }

        Ok(())
    }

    /// Download voice audio from Telegram's file API.
    ///
    /// Returns the raw audio bytes, or sends an error to the user and returns `None`.
    async fn download_voice_audio(
        telegram: &TelegramChannel,
        chat_id: &str,
        file_id: &str,
        file_size: Option<u64>,
    ) -> Option<Vec<u8>> {
        if let Some(size) = file_size
            && size > MAX_VOICE_BYTES as u64
        {
            let _ = telegram
                .send_message(
                    chat_id,
                    "Sorry, that voice message is too large to process.",
                )
                .await;
            return None;
        }

        let file_path = match telegram.get_file(file_id).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "failed to get voice file info");
                let _ = telegram
                    .send_message(chat_id, "Sorry, I could not process your voice message.")
                    .await;
                return None;
            }
        };

        match telegram.download_file(&file_path, MAX_VOICE_BYTES).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!(error = %e, "failed to download voice file");
                let _ = telegram
                    .send_message(chat_id, "Sorry, I could not process your voice message.")
                    .await;
                None
            }
        }
    }

    /// Handle a voice message: download, transcribe, and return the text.
    ///
    /// Returns `None` if an error occurred (error message already sent to user).
    async fn handle_voice_message(
        telegram: &TelegramChannel,
        payload: &VoicePayload,
        whisper: Option<GroqWhisperClient>,
    ) -> Option<String> {
        let Some(whisper_client) = whisper else {
            let _ = telegram
                .send_message(
                    &payload.chat_id,
                    "Sorry, voice transcription is not configured.",
                )
                .await;
            return None;
        };

        let audio_bytes = Self::download_voice_audio(
            telegram,
            &payload.chat_id,
            &payload.file_id,
            payload.file_size,
        )
        .await?;

        match whisper_client
            .transcribe_bytes(audio_bytes, "voice.ogg")
            .await
        {
            Ok(result) => Some(result.text),
            Err(e) => {
                tracing::error!(error = %e, "voice transcription failed");
                let _ = telegram
                    .send_message(
                        &payload.chat_id,
                        "Sorry, I could not transcribe your voice message.",
                    )
                    .await;
                None
            }
        }
    }

    /// Build a Groq Whisper client from voice configuration.
    ///
    /// The `allow_insecure` parameter controls whether non-HTTPS URLs and
    /// non-Groq hosts are accepted. In production, this is read from the
    /// `QUECTO_ALLOW_INSECURE_TELEGRAM_API_BASE` environment variable by the
    /// caller (`run_telegram_polling`).
    pub(super) fn build_whisper_client(
        voice_config: &crate::infrastructure::config::VoiceConfig,
        allow_insecure: bool,
    ) -> Option<GroqWhisperClient> {
        if voice_config.groq.api_key.is_empty() {
            return None;
        }

        if voice_config.groq.api_base.is_empty() {
            return Some(GroqWhisperClient::new(&voice_config.groq.api_key));
        }

        let parsed = reqwest::Url::parse(&voice_config.groq.api_base).ok();
        let Some(url) = parsed else {
            tracing::warn!("invalid voice API base URL, disabling voice transcription");
            return None;
        };
        let valid_scheme = url.scheme() == "https" || (allow_insecure && url.scheme() == "http");
        let valid_host = allow_insecure || url.host_str() == Some("api.groq.com");
        let clean_url = url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none();
        if !(valid_scheme && valid_host && clean_url) {
            tracing::warn!("voice API base URL rejected by security policy");
            None
        } else {
            Some(GroqWhisperClient::with_base_url(
                &voice_config.groq.api_key,
                &voice_config.groq.api_base,
            ))
        }
    }
}
