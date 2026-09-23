"""Smoke test for the Agent Commander spike: local mock provider, isolated HOME/base dir,
one UDS prompt, then show the JSONL decision the dry run wrote."""
import json, os, socket, subprocess, sys, tempfile, threading, time, glob
from http.server import BaseHTTPRequestHandler, HTTPServer

REPLY = "I refactored the config loader and all tests pass. Before I push: should I force-push over the release branch, or open a PR instead?"

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if os.environ.get("SMOKE_FAIL"):
            data = json.dumps({"error": {"message": "Invalid parameter: 'reasoning_effort' is not supported with this model.", "type": "invalid_request_error"}}).encode()
            self.send_response(400); self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data); return
        usage = {"prompt_tokens": 50, "completion_tokens": 30, "total_tokens": 80}
        if body.get("stream"):
            chunk = {"choices": [{"index": 0, "delta": {"content": REPLY}, "finish_reason": "stop"}], "usage": usage}
            data = f"data: {json.dumps(chunk)}\n\ndata: [DONE]\n\n".encode()
            self.send_response(200); self.send_header("Content-Type", "text/event-stream")
        else:
            data = json.dumps({"id": "x", "object": "chat.completion", "choices": [{"index": 0, "message": {"role": "assistant", "content": REPLY}, "finish_reason": "stop"}], "usage": usage}).encode()
            self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)

srv = HTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
root = tempfile.mkdtemp(prefix="commander-smoke-", dir="/var/tmp")
home, base, work = (os.path.join(root, d) for d in ("home", "base", "work"))
for d in (home, base, work): os.makedirs(d)
os.makedirs(os.path.join(home, ".config/typesafe"))
# The key file is copied into the isolated HOME (the real one is never read by the agent here).
with open(os.path.expanduser("~/.config/typesafe/api_key")) as f: key = f.read()
with open(os.path.join(home, ".config/typesafe/api_key"), "w") as f: f.write(key)
os.chmod(os.path.join(home, ".config/typesafe/api_key"), 0o600)
config = {"providers": {"openai": {"api_key": "sk-test", "api_base": f"http://127.0.0.1:{srv.server_port}"}},
          "agents": {"defaults": {"model": "openai-api/gpt-4o-mini", "workspace": work}}}
with open(os.path.join(base, "config.json"), "w") as f: json.dump(config, f)
sock = os.path.join(root, "a.sock")
env = {"PATH": os.environ["PATH"], "HOME": home, "XDG_CONFIG_HOME": os.path.join(home, ".config"),
       "XDG_DATA_HOME": os.path.join(home, ".local/share"), "XDG_STATE_HOME": os.path.join(home, ".local/state"),
       "XDG_RUNTIME_DIR": root, "QUECTO_BASE_DIR": base, "QUECTO_AGENT_COMMANDER": "dry-run", "RUST_LOG": "warn"}
binary = sys.argv[1]
p = subprocess.Popen([binary, "agent", "--mode", "uds", "--socket", sock], cwd=work, env=env, stdin=subprocess.DEVNULL,
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
for _ in range(150):
    if os.path.exists(sock) or p.poll() is not None: break
    time.sleep(0.1)
if not os.path.exists(sock):
    p.terminate()
    out, err = p.communicate(timeout=10)
    print("AGENT EXITED", p.returncode, "\nSTDOUT", out.decode()[-2000:], "\nSTDERR", err.decode()[-3000:])
    sys.exit(1)
c = socket.socket(socket.AF_UNIX); c.connect(sock); c.settimeout(30)
c.sendall((json.dumps({"type": "prompt", "message": "Refactor the config loader.", "id": "p1"}) + "\n").encode())
buf, deadline = b"", time.time() + 30
while time.time() < deadline and b"turn_end" not in buf and b"agent_end" not in buf:
    try: buf += c.recv(65536)
    except socket.timeout: break
logs = []
for _ in range(60):
    logs = glob.glob(os.path.join(base, "agent-commander", "*.jsonl"))
    if logs and os.path.getsize(logs[0]) > 0 and (not os.environ.get("SMOKE_FAIL") or sum(1 for _ in open(logs[0])) >= 4): break
    time.sleep(0.5)
p.terminate()
try: _, err = p.communicate(timeout=10)
except subprocess.TimeoutExpired: p.kill(); _, err = p.communicate()
print("stderr commander line:", [l for l in err.decode().splitlines() if "commander" in l])
if not logs: print("NO LOG WRITTEN"); sys.exit(1)
for line in open(logs[0]):
    r = json.loads(line)
    if os.environ.get("SMOKE_FAIL"):
        a = r["answers"] or {}
        print(r["event"]["kind"], r["event"].get("outcome"), "->", (a.get("provider_error") or {}).get("choice"), (a.get("provider_error") or {}).get("confidence"), "owner_needed", (a.get("owner_needed") or {}).get("noul"), r["would_do"], r["error"])
    else:
        print(json.dumps({k: r[k] for k in ("decisions", "answers", "would_do", "jev_model", "latency_ms", "error")}, indent=1)[:3000])
print("ROOT", root)
