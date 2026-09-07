#!/usr/bin/env python3
"""EXPLICIT FAKE OpenAI Chat Completions server for the swarm spike.

No model inference, credentials, or external calls. Real quecto agent processes
receive deterministic tool calls. This tests tool plumbing, NOT model behavior.
Run: python3 scripts/swarm-spike-fake-provider.py --port 8765
Configure an OpenAI-compatible provider api_base=http://127.0.0.1:8765/v1,
a dummy API key, and model=swarm-spike-fake (force Chat Completions).
Supports JSON and SSE; deliberately does not implement the Responses API.
The current runner's fixed worker prompt supplies worker identity and DB path.
"""
import argparse
import json
import re
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL = "swarm-spike-fake"
USAGE = {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}


def content_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(p.get("text", "") for p in content if isinstance(p, dict))
    return ""


def reply(body):
    messages = body.get("messages", [])
    # Stateless: a completed tool turn ends this deterministic worker run.
    if any(m.get("role") == "tool" for m in messages):
        return {"role": "assistant", "content": "EXPLICIT FAKE provider: worker tool turn ended; inspect tool output and board for success."}, "stop"
    prompt = "\n".join(content_text(m.get("content")) for m in messages if m.get("role") == "user")
    member = re.search(r"You are (worker-\d+), fixed swarm worker", prompt)
    db = re.search(r"s=swarm\.Swarm\((\"(?:[^\"\\]|\\.)*\")\)", prompt)
    tools = {t.get("function", {}).get("name") for t in body.get("tools", [])}
    if not member or not db or "swarm" not in tools:
        raise ValueError("expected runner fixed worker prompt and advertised swarm tool")
    path = json.loads(db.group(1))
    worker = member.group(1)
    code = f'''import swarm, json, time
s = swarm.Swarm({path!r})
member = {worker!r}
workers = sorted(m for m in s.summary()['members'] if m != 'coordinator')
for peer in workers:
    if peer != member:
        s.message(member, peer, 'EXPLICIT FAKE hello from ' + member)
print(json.dumps(s.messages(member)))
finished = []
# Deterministic partition ensures multiple actual agents contribute, not just
# whichever starts first. Fixed sleeps expose overlapping work; no busy polling.
for n in range(1, 6):
    if workers[(n-1) % len(workers)] != member:
        continue
    c = s.claim(member, str(n))
    if c is None:
        break
    task_id = c['task_id']
    n = int(task_id)
    time.sleep(1)
    s.finish(member, task_id, c['claim_token'], evidence=[{{'n': n, 'square': n*n}}], tokens_used=0)
    s.message(member, 'coordinator', 'EXPLICIT FAKE task ' + task_id + ' result ' + str(n*n))
    finished.append(task_id)
print(json.dumps({{'fake_provider': True, 'worker': member, 'finished': finished}}))
'''
    call = {"id": "call_fake_" + worker, "type": "function", "function": {"name": "swarm", "arguments": json.dumps({"op": "run", "code": code})}}
    return {"role": "assistant", "content": None, "tool_calls": [call]}, "tool_calls"


class Handler(BaseHTTPRequestHandler):
    def send_json(self, status, value):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        if self.path.rstrip("/") in ("/v1/models", "/models"):
            self.send_json(200, {"object": "list", "data": [{"id": MODEL, "object": "model", "created": 0, "owned_by": "explicit-fake"}]})
        else:
            self.send_json(404, {"error": {"message": "Explicit fake: endpoint unsupported"}})

    def do_POST(self):
        if self.path.rstrip("/") not in ("/v1/chat/completions", "/chat/completions"):
            self.send_json(404, {"error": {"message": "Explicit fake supports Chat Completions only, not Responses"}})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 8 * 1024 * 1024:
                raise ValueError("request size outside 1..8MiB")
            body = json.loads(self.rfile.read(length))
            message, reason = reply(body)
        except (ValueError, TypeError, AttributeError) as error:
            self.send_json(400, {"error": {"message": str(error)}})
            return
        # Delay every inference response for deterministic runner cancellation tests.
        time.sleep(self.server.delay_seconds)
        base = {"id": "chatcmpl-explicit-fake", "created": 0, "model": MODEL}
        if not body.get("stream"):
            self.send_json(200, {**base, "object": "chat.completion", "choices": [{"index": 0, "message": message, "finish_reason": reason}], "usage": USAGE})
            return
        delta = dict(message)
        if "tool_calls" in delta:
            delta["tool_calls"] = [{"index": i, **call} for i, call in enumerate(delta["tool_calls"])]
        chunks = [
            {**base, "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
            {**base, "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {}, "finish_reason": reason}]},
            {**base, "object": "chat.completion.chunk", "choices": [], "usage": USAGE},
        ]
        data = ("".join("data: " + json.dumps(chunk) + "\n\n" for chunk in chunks) + "data: [DONE]\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument('--delay-seconds', type=float, default=0, help='delay each inference response (0..600) for cancellation tests')
    args = parser.parse_args()
    if not 0 <= args.delay_seconds <= 600:
        parser.error('--delay-seconds must be 0..600')
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.delay_seconds = args.delay_seconds
    print(f"EXPLICIT FAKE provider: http://127.0.0.1:{server.server_port}/v1 ; no model inference", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
