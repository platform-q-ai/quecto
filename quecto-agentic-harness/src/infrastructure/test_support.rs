//! Shared transport helpers for unit and BDD mock servers.

use std::os::unix::net::UnixStream;

/// Read one production-format framed JSON command from a synchronous fixture.
///
/// The fixture keeps its blocking stream for legacy NDJSON replies while a
/// cloned descriptor is converted to Tokio nonblocking mode and driven through
/// the production frame reader on a local current-thread runtime.
pub fn read_framed_command(stream: &UnixStream) -> Option<String> {
    let cloned = stream.try_clone().expect("clone mock UDS stream");
    cloned
        .set_nonblocking(true)
        .expect("set mock UDS stream nonblocking");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .expect("build mock UDS runtime");
    runtime.block_on(async move {
        let stream = tokio::net::UnixStream::from_std(cloned).expect("convert mock UDS stream");
        let mut reader = tokio::io::BufReader::new(stream);
        read_framed_command_async(&mut reader).await
    })
}

/// Read and validate one UTF-8 JSON command through the production frame API.
pub async fn read_framed_command_async<R>(reader: &mut R) -> Option<String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let payload = quecto_line_io::read_frame(reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
        .await
        .expect("read framed mock command")?;
    let message = String::from_utf8(payload).expect("framed command must be UTF-8");
    serde_json::from_str::<serde_json::Value>(&message).expect("framed command must be JSON");
    Some(message)
}

/// Message contents of a `get_messages`-shaped payload, in wire order. Shared
/// by the paged-history test suites (#1061) so page assertions and their panic
/// messages read identically everywhere.
pub fn message_contents(data: &serde_json::Value) -> Vec<String> {
    data["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|m| m["content"].as_str().expect("content string").to_string())
        .collect()
}

#[cfg(test)]
#[path = "test_support_tests.rs"]
mod tests;
