#!/usr/bin/env python3
"""Replay a captured stream.jsonl and project it onto quecto's sub-agent protocol shapes
(get_state state, messages with ordinals, report, session stats, end). Disposable spike code."""
import json, sys

def project(lines):
    state, gen, msgs, by_msg_id, stats, report, end, rate = "starting", 0, [], {}, {}, None, None, None
    timeline = []
    def bump(s):
        nonlocal state, gen
        if s != state: state, gen = s, gen + 1; timeline.append((gen, s))
    for line in lines:
        e = json.loads(line); t = e.get("type"); st = e.get("subtype")
        if t == "system" and st == "init":
            stats["model"] = e.get("model"); stats["sessionKey"] = e.get("session_id"); bump("thinking")
        elif t == "system" and st == "thinking_tokens":
            bump("thinking")
        elif t == "assistant":
            m = e["message"]; mid = m["id"]
            if mid not in by_msg_id:        # one API message arrives as N events, one content block each
                by_msg_id[mid] = {"ordinal": len(msgs), "id": e["uuid"], "role": "assistant", "content": "",
                                  "toolCalls": [], "thinking": None, "apiMessageId": mid}
                msgs.append(by_msg_id[mid])
            q = by_msg_id[mid]
            for b in m.get("content", []):
                if b["type"] == "text": q["content"] += b["text"]; bump("streaming")
                elif b["type"] == "thinking": q["thinking"] = b.get("thinking") or "<redacted: signature only>"
                elif b["type"] == "tool_use":
                    q["toolCalls"].append({"id": b["id"], "name": b["name"], "arguments": b["input"]}); bump("runningTool")
        elif t == "user":
            c = e["message"]["content"]
            for b in (c if isinstance(c, list) else [{"type": "text", "text": c}]):
                if b.get("type") == "tool_result":
                    txt = b["content"] if isinstance(b["content"], str) else "".join(x.get("text", "") for x in b["content"])
                    msgs.append({"ordinal": len(msgs), "id": e["uuid"], "role": "tool", "toolCallId": b["tool_use_id"],
                                 "content": txt, "isError": bool(b.get("is_error"))})
                    bump("thinking")
                else:   # only seen with --replay-user-messages
                    msgs.append({"ordinal": len(msgs), "id": e.get("uuid"), "role": "user", "content": b.get("text")})
        elif t == "rate_limit_event":
            rate = e["rate_limit_info"]
        elif t == "result":
            u = e.get("usage", {}); mu = next(iter((e.get("modelUsage") or {}).values()), {})
            # modelUsage + total_cost_usd are CUMULATIVE for the process; usage is PER RESULT (per user turn)
            stats["tokens"] = {"input": mu.get("inputTokens"), "output": mu.get("outputTokens"),
                               "cacheRead": mu.get("cacheReadInputTokens"), "cacheWrite": mu.get("cacheCreationInputTokens")}
            stats["costMicroUsd"] = round((e.get("total_cost_usd") or 0) * 1e6)
            stats["lastTurnUsage"] = {"input": u.get("input_tokens"), "output": u.get("output_tokens")}
            last_asst = next((m for m in reversed(msgs) if m["role"] == "assistant" and m["content"]), None)
            report = {"messageId": last_asst["id"] if last_asst else None, "ordinal": last_asst["ordinal"] if last_asst else None,
                      "content": e.get("result"), "fullLengthBytes": len((e.get("result") or "").encode())}
            end = {"subtype": st, "is_error": e.get("is_error"), "stop_reason": e.get("stop_reason"),
                   "terminal_reason": e.get("terminal_reason"), "api_error_status": e.get("api_error_status"),
                   "permission_denials": len(e.get("permission_denials") or []), "num_turns": e.get("num_turns")}
            bump("idle")
    return {"timeline": timeline, "messages": msgs, "report": report, "stats": stats, "end": end, "rateLimit": rate}

if __name__ == "__main__":
    out = project(open(sys.argv[1]))
    for m in out["messages"]:
        for k in ("content",):
            if isinstance(m.get(k), str) and len(m[k]) > 120: m[k] = m[k][:120] + "..."
    print(json.dumps(out, indent=1))
