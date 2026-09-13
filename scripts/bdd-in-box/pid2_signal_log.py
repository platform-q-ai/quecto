#!/usr/bin/env python3
"""Signal-logging pid 2 for the #1925 / #1940 in-container proof.

Runs as the container's main process under `--init` (so it is pid 2, the
place a swarm coordinator's harness would be). Blocks SIGTERM/SIGINT/SIGHUP,
runs the BDD suite as its child, and logs every signal it receives with the
sender pid and the sender's cmdline (`sigwaitinfo` loop on a thread). Exits
with the child's status; the log ends with a `SIGNALS_TO_PID2 <n>` line.
"""
import os
import signal
import subprocess
import sys
import threading
import time

LOG = os.environ.get("PID2_SIGNAL_LOG", "/home/dev/pid2-signals.log")
WATCHED = [signal.SIGTERM, signal.SIGINT, signal.SIGHUP]
count = 0
lock = threading.Lock()


def log(line):
    with open(LOG, "a") as f:
        f.write(f"{time.time():.3f} {line}\n")
        f.flush()


def cmdline(pid):
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\0", b" ").decode(errors="replace").strip()
    except OSError as e:
        return f"<unreadable: {e}>"


def watcher():
    global count
    while True:
        info = signal.sigwaitinfo(WATCHED)
        with lock:
            count += 1
        log(
            f"SIGNAL signo={info.si_signo} si_pid={info.si_pid} si_uid={info.si_uid} "
            f"sender_cmdline={cmdline(info.si_pid)!r}"
        )


def main():
    signal.pthread_sigmask(signal.SIG_BLOCK, WATCHED)
    log(f"pid2 wrapper started as pid {os.getpid()} ppid {os.getppid()} argv={sys.argv[1:]!r}")
    threading.Thread(target=watcher, daemon=True).start()
    # The mask is inherited across fork/exec: unblock in the child so the
    # suite and every harness it launches see signals normally.
    child = subprocess.Popen(
        sys.argv[1:],
        preexec_fn=lambda: signal.pthread_sigmask(signal.SIG_UNBLOCK, WATCHED),
    )
    log(f"child pid {child.pid}")
    status = child.wait()
    log(f"child exited status={status}")
    with lock:
        log(f"SIGNALS_TO_PID2 {count}")
    sys.exit(status if status >= 0 else 128 - status)


if __name__ == "__main__":
    main()
