//! Unix-socket fixtures: a stdio bridge (a `socket_proxy` argv) and a
//! stand-in child's listener.
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A first chunk holding this is a retried connection (the slow-accept
/// marker's contract).
const RETRY: &[u8] = b"PROXY_RETRY_MARKER";

/// One read's size, as each pump reads.
const CHUNK: usize = 65_536;

/// Whether `fd` becomes readable within `wait`.
fn readable(fd: i32, wait: Duration) -> bool {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: one valid pollfd, for the duration of the call.
    let ready = unsafe { libc::poll(&mut poll, 1, millis) };
    ready > 0
}

/// The bridge's options.
struct Bridge {
    socket: PathBuf,
    /// On stdin's EOF, close the whole connection and exit (the parent is
    /// gone: the child must see its bound connection lost), rather than
    /// only shutting the write half and draining the child's answer.
    close_on_stdin_eof: bool,
    /// While this file exists, a first chunk of stdin holding
    /// `PROXY_RETRY_MARKER` makes the bridge remove it and fail, as a proxy
    /// whose first connection is refused.
    slow_accept_marker: Option<PathBuf>,
}

fn bridge_options(args: &[String]) -> Result<Bridge, String> {
    let mut options = Bridge {
        socket: PathBuf::new(),
        close_on_stdin_eof: false,
        slow_accept_marker: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--close-on-stdin-eof" => options.close_on_stdin_eof = true,
            "--slow-accept-marker" => {
                let marker = args.next().ok_or("--slow-accept-marker takes a path")?;
                options.slow_accept_marker = Some(PathBuf::from(marker));
            }
            socket if options.socket.as_os_str().is_empty() => {
                options.socket = PathBuf::from(socket);
            }
            other => return Err(format!("uds-bridge: an extra argument {other}")),
        }
    }
    match options.socket.as_os_str().is_empty() {
        true => Err("uds-bridge: a socket path".to_owned()),
        false => Ok(options),
    }
}

/// `uds-bridge`: pumps stdin into the socket and the socket into stdout
/// until the socket's EOF.
pub fn bridge(args: &[String]) -> Result<(), String> {
    let options = bridge_options(args)?;
    let socket = UnixStream::connect(&options.socket)
        .map_err(|error| format!("connect {}: {error}", options.socket.display()))?;
    let mut prefetched = Vec::new();
    if let Some(marker) = options.slow_accept_marker.as_deref().filter(|m| m.exists()) {
        let stdin = std::io::stdin();
        if readable(stdin.as_raw_fd(), Duration::from_millis(200)) {
            let mut chunk = vec![0; CHUNK];
            let read = stdin
                .lock()
                .read(&mut chunk)
                .map_err(|e| format!("stdin: {e}"))?;
            prefetched.extend_from_slice(&chunk[..read]);
            if prefetched
                .windows(RETRY.len())
                .any(|window| window == RETRY)
            {
                let _gone = std::fs::remove_file(marker);
                return Err("a retried connection is refused".to_owned());
            }
        }
    }
    let mut writer = socket
        .try_clone()
        .map_err(|error| format!("clone: {error}"))?;
    let close_on_eof = options.close_on_stdin_eof;
    std::thread::spawn(move || {
        if !prefetched.is_empty() && writer.write_all(&prefetched).is_err() {
            return;
        }
        let mut stdin = std::io::stdin().lock();
        let mut chunk = vec![0; CHUNK];
        loop {
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    let how = match close_on_eof {
                        true => std::net::Shutdown::Both,
                        false => std::net::Shutdown::Write,
                    };
                    let _closed = writer.shutdown(how);
                    if close_on_eof {
                        std::process::exit(0);
                    }
                    return;
                }
                Ok(read) => {
                    if writer.write_all(&chunk[..read]).is_err() {
                        return;
                    }
                }
            }
        }
    });
    let mut reader = socket;
    let mut stdout = std::io::stdout().lock();
    let mut chunk = vec![0; CHUNK];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if stdout
                    .write_all(&chunk[..read])
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    break;
                }
            }
        }
    }
    std::process::exit(0);
}

/// The listener's options.
struct Listen {
    socket: PathBuf,
    /// Remove a file already at the path first.
    unlink: bool,
    /// Log `decoy-listening` once bound and `decoy-connection` per
    /// connection, as JSON lines, here.
    decoy_log: Option<PathBuf>,
    /// Accept this many connections (closing each), then linger and exit;
    /// none: accept until idle.
    accepts: Option<usize>,
    linger: Duration,
    /// Keep every accepted connection open until exit.
    hold: bool,
    /// Exit when no connection arrives for this long; none: never.
    idle_exit: Option<Duration>,
}

fn seconds(value: Option<&String>, flag: &str) -> Result<Duration, String> {
    let text = value.ok_or(format!("{flag} takes seconds"))?;
    let seconds: f64 = text
        .parse()
        .map_err(|_| format!("{flag}: seconds, not {text}"))?;
    Duration::try_from_secs_f64(seconds).map_err(|_| format!("{flag}: seconds, not {text}"))
}

fn listen_options(args: &[String]) -> Result<Listen, String> {
    let mut options = Listen {
        socket: PathBuf::new(),
        unlink: false,
        decoy_log: None,
        accepts: None,
        linger: Duration::ZERO,
        hold: false,
        idle_exit: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--unlink" => options.unlink = true,
            "--hold" => options.hold = true,
            "--decoy-log" => {
                let log = args.next().ok_or("--decoy-log takes a path")?;
                options.decoy_log = Some(PathBuf::from(log));
            }
            "--accepts" => {
                let count = args.next().ok_or("--accepts takes a count")?;
                options.accepts = Some(count.parse().map_err(|_| format!("--accepts {count}"))?);
            }
            "--linger" => options.linger = seconds(args.next(), "--linger")?,
            "--idle-exit" => options.idle_exit = Some(seconds(args.next(), "--idle-exit")?),
            socket if options.socket.as_os_str().is_empty() => {
                options.socket = PathBuf::from(socket);
            }
            other => return Err(format!("uds-listen: an extra argument {other}")),
        }
    }
    match options.socket.as_os_str().is_empty() {
        true => Err("uds-listen: a socket path".to_owned()),
        false => Ok(options),
    }
}

fn log(file: &Path, kind: &str, path: &Path) -> Result<(), String> {
    let record = serde_json::json!({"kind": kind, "path": path.display().to_string()});
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|error| format!("open {}: {error}", file.display()))?;
    writeln!(log, "{record}").map_err(|error| format!("log: {error}"))
}

/// `uds-listen`: binds the socket and accepts as the options say.
pub fn listen(args: &[String]) -> Result<(), String> {
    let options = listen_options(args)?;
    if options.unlink {
        match std::fs::remove_file(&options.socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("unlink: {error}")),
        }
    }
    let listener = UnixListener::bind(&options.socket)
        .map_err(|error| format!("bind {}: {error}", options.socket.display()))?;
    if let Some(file) = &options.decoy_log {
        log(file, "decoy-listening", &options.socket)?;
    }
    let mut held = Vec::new();
    let mut accepted = 0;
    while options.accepts.is_none_or(|limit| accepted < limit) {
        if let Some(idle) = options.idle_exit
            && !readable(listener.as_raw_fd(), idle)
        {
            break;
        }
        let (connection, _) = listener
            .accept()
            .map_err(|error| format!("accept: {error}"))?;
        accepted += 1;
        if let Some(file) = &options.decoy_log {
            log(file, "decoy-connection", &options.socket)?;
        }
        if options.hold {
            held.push(connection);
        }
    }
    assert!(
        options.accepts.is_none_or(|limit| accepted == limit) || options.idle_exit.is_some(),
        "the listener leaves only at its count or when idle"
    );
    assert!(
        !options.hold || held.len() == accepted,
        "every connection held"
    );
    std::thread::sleep(options.linger);
    drop(held);
    Ok(())
}
