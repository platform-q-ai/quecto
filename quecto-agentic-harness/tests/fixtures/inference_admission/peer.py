"""Test-only reverse stdio bridge; not an official runtime/container adapter."""
import json
import os
import socket
import struct
import subprocess
import sys

CAP = 4096


def read_frame(stream):
    def exact(n):
        data = b""
        while len(data) < n:
            part = stream.read(n - len(data))
            if not part:
                raise EOFError("partial frame")
            data += part
        return data
    size = struct.unpack(">I", exact(4))[0]
    if size > CAP + 1:
        raise ValueError("oversized bridge frame")
    return exact(size)


def write_frame(stream, data):
    stream.write(struct.pack(">I", len(data)) + data)
    stream.flush()


def main():
    mode = sys.argv[1]
    if mode == "proxy":
        # Nested child inherits only the dedicated endpoint and scoped capability.
        child = subprocess.Popen([sys.executable, "-I", __file__, "nested"],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 env=dict(os.environ))
        try:
            write_frame(child.stdin, read_frame(sys.stdin.buffer))
            request = read_frame(child.stdout)
            with socket.socket(socket.AF_UNIX) as sock:
                sock.settimeout(3)
                sock.connect(os.environ["ADMISSION_ENDPOINT"])
                wire = sock.makefile("rwb", buffering=0)
                write_frame(wire, request)
                write_frame(child.stdin, read_frame(wire))
            write_frame(sys.stdout.buffer, read_frame(child.stdout))
            if child.wait(timeout=3) != 0:
                raise RuntimeError("nested child failed")
        finally:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=3)
        return
    request = json.loads(read_frame(sys.stdin.buffer))
    request["peer"] = {"pid": os.getpid(), "ppid": os.getppid(),
                       "argv": sys.argv[1:], "env": dict(os.environ)}
    payload = json.dumps(request).encode()
    if request["id"] == "malformed":
        payload = b"{broken"
    elif request["id"] == "oversized":
        payload = b"x" * (CAP + 1)
    if mode == "nested":
        write_frame(sys.stdout.buffer, payload)
        reply = read_frame(sys.stdin.buffer)
    else:
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(3)
            sock.connect(os.environ["ADMISSION_ENDPOINT"])
            wire = sock.makefile("rwb", buffering=0)
            write_frame(wire, payload)
            reply = read_frame(wire)
    write_frame(sys.stdout.buffer, reply)


if __name__ == "__main__":
    main()
