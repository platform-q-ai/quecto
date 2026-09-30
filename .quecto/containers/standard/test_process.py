"""Bounded fixture execution: hard-kill the entire process group on timeout."""
import os
import signal
import subprocess


def run_fixture(argv, *, env, timeout=5, check=False, capture_output=True, text=False):
    assert timeout > 0
    assert capture_output
    process = subprocess.Popen(
        argv, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=text, start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass  # The complete group already exited before the kill.
        stdout, stderr = process.communicate()
        raise subprocess.TimeoutExpired(argv, timeout, output=stdout, stderr=stderr)
    result = subprocess.CompletedProcess(argv, process.returncode, stdout, stderr)
    if check:
        result.check_returncode()
    return result
