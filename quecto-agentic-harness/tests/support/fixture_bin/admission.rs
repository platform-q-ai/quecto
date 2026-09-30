//! The inference-admission transport experiment's peer (#1679 P0): a
//! test-only reverse stdio bridge, not an official runtime or container
//! adapter. Frames are a 4-byte big-endian length and the bytes, at most
//! [`CAP`] + 1 of them.
//!
//! - `direct`: reads the request frame on stdin, adds its `peer` record
//!   (pid, ppid, argv after the fixture name, environment), sends it to
//!   `$ADMISSION_ENDPOINT` and writes the reply frame to stdout.
//! - `proxy`: starts a `nested` peer (itself, with the same environment),
//!   carries the request to it, the nested request to the endpoint, the
//!   endpoint's reply back to it, and its reply to stdout.
//! - `nested`: as `direct`, but talks to its parent over stdio instead of
//!   the endpoint.
//!
//! A request id `malformed` or `oversized` sends a broken or oversized
//! payload instead, for the listener's rejections.
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::Duration;

const CAP: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(3);

fn read_frame(stream: &mut impl Read) -> Result<Vec<u8>, String> {
    let mut size = [0; 4];
    stream
        .read_exact(&mut size)
        .map_err(|error| format!("partial frame: {error}"))?;
    let size = usize::try_from(u32::from_be_bytes(size)).map_err(|_| "a frame size")?;
    if size > CAP + 1 {
        return Err("oversized bridge frame".to_owned());
    }
    let mut data = vec![0; size];
    stream
        .read_exact(&mut data)
        .map_err(|error| format!("partial frame: {error}"))?;
    Ok(data)
}

fn write_frame(stream: &mut impl Write, data: &[u8]) -> Result<(), String> {
    let size = u32::try_from(data.len()).map_err(|_| "a frame too long")?;
    stream
        .write_all(&size.to_be_bytes())
        .and_then(|()| stream.write_all(data))
        .and_then(|()| stream.flush())
        .map_err(|error| format!("write a frame: {error}"))
}

/// A connection to `$ADMISSION_ENDPOINT`, made within [`TIMEOUT`] (as
/// Python's `settimeout(3)` bounded its `connect`), whose reads and writes
/// are bounded too.
fn endpoint() -> Result<UnixStream, String> {
    let path = std::env::var_os("ADMISSION_ENDPOINT").ok_or("ADMISSION_ENDPOINT")?;
    let (sender, connected) = std::sync::mpsc::channel();
    let target = path.clone();
    std::thread::spawn(move || {
        let _gone = sender.send(UnixStream::connect(&target));
    });
    let stream = match connected.recv_timeout(TIMEOUT) {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => return Err(format!("connect {}: {error}", path.to_string_lossy())),
        Err(_) => return Err(format!("connect {}: timed out", path.to_string_lossy())),
    };
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|error| format!("timeouts: {error}"))?;
    Ok(stream)
}

/// The nested peer: killed and reaped whenever the proxy leaves, however
/// it leaves (Python's `finally`).
struct Nested(std::process::Child);

impl Nested {
    /// The nested peer's exit, waited for within [`TIMEOUT`].
    fn wait(&mut self) -> Result<std::process::ExitStatus, String> {
        let deadline = std::time::Instant::now() + TIMEOUT;
        loop {
            match self.0.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => return Err("the nested peer did not exit in time".to_owned()),
                Err(error) => return Err(format!("wait: {error}")),
            }
        }
    }
}

impl Drop for Nested {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _killed = self.0.kill();
        }
        let _reaped = self.0.wait();
    }
}

/// The request with this peer's record, or the broken or oversized
/// payload its id asks for.
fn payload(request: &[u8], mode: &str) -> Result<Vec<u8>, String> {
    let mut request: serde_json::Value =
        serde_json::from_slice(request).map_err(|error| format!("a request: {error}"))?;
    // SAFETY: `getppid` has no preconditions and cannot fail.
    let ppid = unsafe { libc::getppid() };
    let env: serde_json::Map<String, serde_json::Value> = std::env::vars_os()
        .map(|(name, value)| {
            let value = value.to_string_lossy().into_owned();
            (
                name.to_string_lossy().into_owned(),
                serde_json::Value::String(value),
            )
        })
        .collect();
    request["peer"] = serde_json::json!({
        "pid": std::process::id(),
        "ppid": ppid,
        "argv": [mode],
        "env": env,
    });
    Ok(match request["id"].as_str() {
        Some("malformed") => b"{broken".to_vec(),
        Some("oversized") => vec![b'x'; CAP + 1],
        _ => serde_json::to_vec(&request).map_err(|error| format!("a payload: {error}"))?,
    })
}

fn proxy() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| format!("the fixture: {error}"))?;
    let mut child = Nested(
        Command::new(exe)
            .args(["admission-peer", "nested"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start the nested peer: {error}"))?,
    );
    let mut to_child = child.0.stdin.take().ok_or("the nested peer's stdin")?;
    let mut from_child = child.0.stdout.take().ok_or("the nested peer's stdout")?;
    write_frame(&mut to_child, &read_frame(&mut std::io::stdin().lock())?)?;
    let request = read_frame(&mut from_child)?;
    let mut wire = endpoint()?;
    write_frame(&mut wire, &request)?;
    write_frame(&mut to_child, &read_frame(&mut wire)?)?;
    write_frame(&mut std::io::stdout().lock(), &read_frame(&mut from_child)?)?;
    drop(to_child);
    match child.wait()?.success() {
        true => Ok(()),
        false => Err("nested child failed".to_owned()),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let mode = match args {
        [mode] => mode.as_str(),
        _ => return Err("admission-peer direct|proxy|nested".to_owned()),
    };
    match mode {
        "proxy" => proxy(),
        "direct" | "nested" => {
            let request = read_frame(&mut std::io::stdin().lock())?;
            let payload = payload(&request, mode)?;
            let reply = match mode {
                "nested" => {
                    write_frame(&mut std::io::stdout().lock(), &payload)?;
                    read_frame(&mut std::io::stdin().lock())?
                }
                _ => {
                    let mut wire = endpoint()?;
                    write_frame(&mut wire, &payload)?;
                    read_frame(&mut wire)?
                }
            };
            write_frame(&mut std::io::stdout().lock(), &reply)
        }
        other => Err(format!("admission-peer: unknown mode {other}")),
    }
}
