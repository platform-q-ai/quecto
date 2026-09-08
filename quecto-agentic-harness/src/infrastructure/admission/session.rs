//! One authority connection: bounded framed reads, correlated replies and
//! server-initiated notices. Malformed or oversized input closes the session.
use quecto_line_io::{FrameError, read_frame, write_frame};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};

use super::protocol::*;
use super::server::Command;

pub(super) async fn serve(
    stream: UnixStream,
    session: u64,
    role: Role,
    commands: mpsc::UnboundedSender<Command>,
) {
    let (read, mut write) = stream.into_split();
    let (notices, mut outbound) = mpsc::unbounded_channel::<Reply>();
    if commands
        .send(Command::Open {
            session,
            role,
            notices: notices.clone(),
        })
        .is_err()
    {
        return;
    }
    let writer = tokio::spawn(async move {
        while let Some(reply) = outbound.recv().await {
            let bytes = serde_json::to_vec(&reply).expect("reply is serializable");
            if write_frame(&mut write, &bytes, FRAME_CAP).await.is_err() {
                break;
            }
        }
        let _ = write.shutdown().await;
    });
    let mut reader = BufReader::new(read);
    loop {
        // Idle connections wait indefinitely; once the first byte of a frame
        // has arrived the remainder must follow within the framing deadline.
        match reader.fill_buf().await {
            Ok(buffer) if buffer.is_empty() => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let frame = match tokio::time::timeout(FRAME_DEADLINE, read_frame(&mut reader, FRAME_CAP))
            .await
        {
            Err(_) => {
                let _ = notices.send(Reply {
                    id: None,
                    body: Body::Error {
                        code: ErrorCode::Malformed,
                        reason: "frame deadline elapsed".into(),
                    },
                });
                break;
            }
            Ok(Ok(Some(frame))) => frame,
            Ok(Ok(None)) => break,
            Ok(Err(FrameError::Oversized { .. })) | Ok(Err(FrameError::VersionMismatch { .. })) => {
                let _ = notices.send(Reply {
                    id: None,
                    body: Body::Error {
                        code: ErrorCode::Malformed,
                        reason: "frame rejected".into(),
                    },
                });
                break;
            }
            Ok(Err(FrameError::Io(_))) => break,
        };
        let request: Request = match serde_json::from_slice(&frame) {
            Ok(request) => request,
            Err(error) => {
                let _ = notices.send(Reply {
                    id: None,
                    body: Body::Error {
                        code: ErrorCode::Malformed,
                        reason: format!("malformed request: {error}"),
                    },
                });
                break;
            }
        };
        let id = request.id;
        let (reply, receiver) = oneshot::channel();
        if commands
            .send(Command::Request {
                session,
                request,
                reply,
            })
            .is_err()
        {
            break;
        }
        let notices = notices.clone();
        tokio::spawn(async move {
            if let Ok(body) = receiver.await {
                let _ = notices.send(Reply { id: Some(id), body });
            }
        });
    }
    let _ = commands.send(Command::Closed { session });
    drop(notices);
    writer.abort();
}
