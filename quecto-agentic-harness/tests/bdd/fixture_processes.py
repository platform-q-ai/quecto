"""Identity-scoped process ownership for script-managed BDD fixtures (Linux)."""
import json
import os
from pathlib import Path
import select
import signal
import sys


def identity(pid):
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()
        return fields[19], fields[0]
    except FileNotFoundError:
        return None


def terminate(record):
    pid, start = record
    try:
        fd = os.pidfd_open(pid)
    except ProcessLookupError:
        return
    try:
        current = identity(pid)
        if current is None or current[0] != start or current[1] == "Z":
            return
        signal.pidfd_send_signal(fd, signal.SIGTERM)
        if not select.select([fd], [], [], 2)[0]:
            signal.pidfd_send_signal(fd, signal.SIGKILL)
            if not select.select([fd], [], [], 2)[0]:
                raise RuntimeError(f"fixture process {pid} did not exit")
    except ProcessLookupError:
        return
    finally:
        os.close(fd)


mode, directory, *args = sys.argv[1:]
root = Path(directory)
root.mkdir(exist_ok=True)
if mode == "track":
    env, pid = args[0], int(args[1])
    current = identity(pid)
    if current is not None:
        # One file per launch: concurrent joins cannot overwrite each other.
        record = root / f"{env}.{pid}.json"
        temporary = record.with_suffix(f".{os.getpid()}.tmp")
        temporary.write_text(json.dumps([pid, current[0]]))
        temporary.replace(record)
elif mode == "live":
    # How many tracked processes of the environment are still alive (not
    # zombies): evidence of whether members ended before a kill ran.
    alive = 0
    for record in root.glob(f"{args[0]}.*.json"):
        try:
            pid, start = json.loads(record.read_text())
        except FileNotFoundError:
            continue
        current = identity(pid)
        if current is not None and current[0] == start and current[1] != "Z":
            alive += 1
    print(alive)
else:
    for record in root.glob(f"{args[0]}.*.json" if args else "*.json"):
        try:
            data = json.loads(record.read_text())
        except FileNotFoundError:
            continue
        terminate(data)
        record.unlink(missing_ok=True)
