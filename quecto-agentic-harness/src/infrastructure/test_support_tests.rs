use super::*;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn async_helper_reads_a_production_frame() {
    let (mut writer, reader) = tokio::io::duplex(128);
    quecto_line_io::write_frame(
        &mut writer,
        br#"{"type":"get_state"}"#,
        quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
    )
    .await
    .unwrap();
    writer.shutdown().await.unwrap();
    let mut reader = tokio::io::BufReader::new(reader);
    assert_eq!(
        read_framed_command_async(&mut reader).await.as_deref(),
        Some(r#"{"type":"get_state"}"#)
    );
}
