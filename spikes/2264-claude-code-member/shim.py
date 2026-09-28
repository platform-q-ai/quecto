#!/usr/bin/env python3
"""Spike shim: one long-lived `claude -p` stream-json process as a swarm worker member.
Usage: shim.py <workdir> [scenario]   scenarios: roundtrip (default) | guards | kill
Disposable spike code: no tests, no polish."""
import json, os, signal, subprocess, sys, threading, time, queue
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

WORK = os.path.abspath(sys.argv[1]); SCEN = sys.argv[2] if len(sys.argv) > 2 else "roundtrip"
SANDBOX = os.path.join(WORK, "sandbox"); os.makedirs(SANDBOX, exist_ok=True)
os.environ["SPIKE_BOARD"] = os.path.join(WORK, "board.json")
import board
board.BOARD = os.environ["SPIKE_BOARD"]
board.fresh(board.BOARD)
MEMBER = "W1"
MODEL = os.environ.get("SPIKE_MODEL", "haiku")
PY = os.environ.get("SPIKE_PY", sys.executable)

env_common = {"SPIKE_BOARD": board.BOARD, "SPIKE_MEMBER": MEMBER, "SPIKE_CWD": SANDBOX,
              "SPIKE_MCP_LOG": os.path.join(WORK, "mcp.log"), "SPIKE_HOOK_LOG": os.path.join(WORK, "hook.log")}
mcp_cfg = {"mcpServers": {"board": {"type": "stdio", "command": PY, "args": [os.path.join(HERE, "board_mcp.py")], "env": env_common}}}
settings = {"hooks": {"PreToolUse": [{"matcher": "Edit|Write|MultiEdit|NotebookEdit|Bash",
             "hooks": [{"type": "command", "command": f"{PY} {os.path.join(HERE, 'hook_pretool.py')}"}]}]}}

cmd = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
       "--model", MODEL, "--tools", "Read,Edit,Write,Bash,Grep,Glob",
       "--mcp-config", json.dumps(mcp_cfg), "--strict-mcp-config",
       "--settings", json.dumps(settings), "--setting-sources", os.environ.get("SPIKE_SETTING_SOURCES", "project"),
       "--permission-mode", os.environ.get("SPIKE_PERM", "bypassPermissions"),
       "--no-session-persistence", "--max-budget-usd", os.environ.get("SPIKE_BUDGET", "0.50")]
if os.environ.get("SPIKE_PERM", "bypassPermissions") == "bypassPermissions":
    cmd.append("--allow-dangerously-skip-permissions")
# Allowlisted env: a parent Claude Code session leaks CLAUDECODE / CLAUDE_CODE_MESSAGING_* / CLAUDE_CODE_SESSION_ID
# into children; pass only what the member needs.
ENV_ALLOW = ("PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "TERM", "TMPDIR", "XDG_RUNTIME_DIR", "CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY")
child_env = {k: os.environ[k] for k in ENV_ALLOW if k in os.environ}
child_env.update(env_common)
# Claude's Bash tool setsid()s every command, so its children leave claude's process group.
# Become a child subreaper so anything orphaned by claude reparents to the shim, then sweep descendants.
import ctypes
PR_SET_CHILD_SUBREAPER = 36
if os.environ.get("SPIKE_SUBREAPER", "1") == "1":
    assert ctypes.CDLL(None, use_errno=True).prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0

def descendants(root):
    kids = {}
    for d in os.listdir("/proc"):
        if d.isdigit():
            try:
                st = open(f"/proc/{d}/stat").read(); ppid = int(st.rsplit(")", 1)[1].split()[1])
            except (OSError, IndexError, ValueError): continue
            kids.setdefault(ppid, []).append(int(d))
    out, stack = [], [root]
    while stack:
        for k in kids.get(stack.pop(), []): out.append(k); stack.append(k)
    return out

def sweep():
    for sig in (signal.SIGTERM, signal.SIGKILL):
        left = [p for p in descendants(os.getpid()) if p != os.getpid()]
        if not left: return
        print(f"[shim] sweeping {len(left)} leftover descendant(s) with {sig.name}: {left}", flush=True)
        for p in left:
            try: os.kill(p, sig)
            except ProcessLookupError: pass
        time.sleep(1.0)
        for p in left:
            try: os.waitpid(p, os.WNOHANG)
            except ChildProcessError: pass
proc = subprocess.Popen(cmd, cwd=SANDBOX, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(os.path.join(WORK, "claude.stderr"), "w"),
                        env=child_env, start_new_session=True, text=True, bufsize=1)
print(f"[shim] claude pid={proc.pid} pgid={os.getpgid(proc.pid)}", flush=True)
events = queue.Queue()
cap = open(os.path.join(WORK, "stream.jsonl"), "w")

def reader():
    for line in proc.stdout:
        cap.write(line); cap.flush()
        try: ev = json.loads(line)
        except ValueError: ev = {"type": "_unparsed", "line": line}
        events.put(ev)
    events.put({"type": "_eof", "rc": proc.wait()})
threading.Thread(target=reader, daemon=True).start()

def kill_tree(sig=signal.SIGTERM, grace=5.0):
    try: os.killpg(proc.pid, sig)
    except ProcessLookupError: return
    end = time.time() + grace
    while time.time() < end and proc.poll() is None: time.sleep(0.1)
    try: os.killpg(proc.pid, signal.SIGKILL)   # sweep group leftovers (SIGKILL does not dump core)
    except ProcessLookupError: pass
    proc.wait(); sweep()

def on_term(signum, frame):
    print(f"[shim] got signal {signum}; killing claude process group {proc.pid}", flush=True)
    # SPIKE_KILLSIG=KILL simulates a claude that cannot run its own shutdown (crash / hard kill)
    kill_tree(signal.SIGKILL if os.environ.get("SPIKE_KILLSIG") == "KILL" else signal.SIGTERM); sys.exit(143)
signal.signal(signal.SIGTERM, on_term)

def say(text):
    msg = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]},
           "parent_tool_use_id": None, "session_id": ""}
    proc.stdin.write(json.dumps(msg) + "\n"); proc.stdin.flush()

def turn(text, timeout=240):
    print(f"\n[shim] >>> {text[:100]}", flush=True); say(text)
    end = time.time() + timeout
    while time.time() < end:
        try: ev = events.get(timeout=1)
        except queue.Empty: continue
        t = ev.get("type")
        if t == "assistant":
            for b in ev["message"].get("content", []):
                if b.get("type") == "text": print(f"[claude] {b['text'][:300]}", flush=True)
                elif b.get("type") == "tool_use": print(f"[tool_use] {b['name']} {json.dumps(b['input'])[:200]}", flush=True)
        elif t == "user":
            for b in ev["message"].get("content", []) if isinstance(ev["message"].get("content"), list) else []:
                if b.get("type") == "tool_result":
                    c = b.get("content"); c = c if isinstance(c, str) else json.dumps(c)
                    print(f"[tool_result]{' ERROR' if b.get('is_error') else ''} {c[:250]}", flush=True)
        elif t == "system":
            print(f"[system:{ev.get('subtype')}] alive pid={proc.pid}", flush=True)
        elif t == "result":
            print(f"[result] subtype={ev.get('subtype')} is_error={ev.get('is_error')} turns={ev.get('num_turns')} "
                  f"cost={ev.get('total_cost_usd')} usage={json.dumps(ev.get('usage'))[:300]}", flush=True)
            return ev
        elif t == "_eof":
            print(f"[shim] claude exited rc={ev['rc']}", flush=True); return ev
        else:
            print(f"[event:{t}] {json.dumps(ev)[:200]}", flush=True)
    print("[shim] turn timeout", flush=True); return None

try:
    if SCEN == "roundtrip":
        turn("You are a swarm worker member W1. Use the board tools: look at the board summary, claim the ready task, "
             "reserve the files you will touch, do the work in your working directory, then submit evidence "
             "(the exact command you ran and its output). Be brief.")
        print(f"[shim] between turns: claude alive={proc.poll() is None}", flush=True)
        board.post(MEMBER, "coordinator", "Please also reply to me (coordinator) via board_send with the greeting text "
                   "your hello.py prints, then ack this message.")
        turn("You have a new board message; read your inbox and act on it. Also tell me: what task id did you submit earlier?")
    elif SCEN == "guards":
        turn("You are swarm worker W1. Without using any board tools, use the Write tool to create notes.txt containing 'hi'. "
             "If a tool call is refused, report the refusal text verbatim and stop; do not retry or work around it.")
        turn("Now run this exact Bash command and report the result verbatim: git push origin HEAD --dry-run . "
             "If it is refused, report the refusal verbatim and stop.")
    elif SCEN == "midturn":
        # A board message arrives while a turn is running: write a second user turn mid-flight.
        def late():
            time.sleep(6); board.post(MEMBER, "coordinator", "Stop what you are doing and reply 'ack-midturn' via board_send.")
            print("[shim] >>> (mid-turn) new board message; read your inbox", flush=True)
            say("You have a new board message; read your inbox and act on it.")
        threading.Thread(target=late, daemon=True).start()
        # Finding: the mid-turn message is folded into the running turn at the next tool boundary -> ONE result.
        turn("Run this exact Bash command in the foreground: sleep 15 && echo slept . Then say 'done sleeping'.")
    elif SCEN == "kill":
        turn("Run this exact Bash command with run_in_background set to true: sleep 300 . Then say 'started'. Do nothing else.")
        print(f"[shim] tree before kill:", flush=True)
        os.system(f"ps -o pid,pgid,sid,cmd --forest -g {proc.pid}")
        print("[shim] waiting for SIGTERM from outside...", flush=True)
        while proc.poll() is None: time.sleep(0.5)
    print("[shim] closing stdin (clean end)", flush=True)
    proc.stdin.close()
    try: rc = proc.wait(timeout=30)
    except subprocess.TimeoutExpired: kill_tree(); rc = proc.returncode
    print(f"[shim] claude exited rc={rc} after stdin EOF", flush=True)
finally:
    if proc.poll() is None: kill_tree()
    print("[shim] final board:", json.dumps(board.read(), indent=1)[:2000], flush=True)
