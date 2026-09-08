#[path = "common/uds_event_reader.rs"]
mod uds_event_reader;

#[cfg(test)]
mod tests {
    use super::uds_event_reader::EventReader;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    #[test]
    fn retains_read_ahead_between_poll_budgets() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(b"token\nagent_end\n").unwrap();
        let mut reader = EventReader::new(reader).unwrap();
        assert_eq!(reader.poll().unwrap().as_deref(), Some("token"));
        assert_eq!(reader.poll().unwrap().as_deref(), Some("agent_end"));
    }

    #[test]
    fn retains_partial_utf8_frame_across_socket_timeout() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        let mut reader = EventReader::new(reader).unwrap();
        writer.write_all(b"token: \xc3").unwrap();
        assert_eq!(reader.poll().unwrap(), None);
        writer.write_all(b"\xa9\nagent_end\n").unwrap();
        assert_eq!(reader.poll().unwrap().as_deref(), Some("token: é"));
        assert_eq!(reader.poll().unwrap().as_deref(), Some("agent_end"));
    }
}
