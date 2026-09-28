"""Stand-in swarm board: a JSON file guarded by flock. Disposable spike code."""
import fcntl, json, os, secrets, time
from contextlib import contextmanager

BOARD = os.environ.get("SPIKE_BOARD", "board.json")

def fresh(path=BOARD):
    data = {
        "tasks": {
            "T1": {"title": "Create hello.py that prints a greeting, then run it with python3",
                   "state": "ready", "claimant": None, "token": None,
                   "reserved": [], "evidence": None},
        },
        "inbox": {},       # member -> [ {id, from, text, acked} ]
        "outbox": [],      # messages members sent (to coordinator etc.)
        "next_msg": 1,
    }
    with open(path, "w") as f:
        json.dump(data, f, indent=2)

@contextmanager
def locked(path=BOARD):
    with open(path + ".lock", "a") as lk:
        fcntl.flock(lk, fcntl.LOCK_EX)
        with open(path) as f:
            data = json.load(f)
        yield data
        tmp = path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(data, f, indent=2)
        os.replace(tmp, path)

def read(path=BOARD):
    with open(path) as f:
        return json.load(f)

def summary(member):
    d = read()
    return {"member": member, "tasks": [{"id": k, "title": t["title"], "state": t["state"],
             "claimant": t["claimant"], "reserved": t["reserved"]} for k, t in d["tasks"].items()],
            "unread": sum(1 for m in d["inbox"].get(member, []) if not m["acked"])}

def claim(member, task_id):
    with locked() as d:
        t = d["tasks"].get(task_id)
        if t is None: return {"error": f"no task {task_id}"}
        if t["state"] != "ready": return {"error": f"task {task_id} is {t['state']}"}
        t.update(state="claimed", claimant=member, token=secrets.token_hex(4))
        return {"task_id": task_id, "token": t["token"]}

def _auth(d, task_id, token):
    t = d["tasks"].get(task_id)
    if t is None: return None, {"error": f"no task {task_id}"}
    if t["token"] != token: return None, {"error": "bad claim token"}
    return t, None

def reserve(task_id, token, files, cwd):
    with locked() as d:
        t, err = _auth(d, task_id, token)
        if err: return err
        abs_files = [os.path.normpath(os.path.join(cwd, f)) for f in files]
        t["reserved"] = sorted(set(t["reserved"]) | set(abs_files))
        return {"reserved": t["reserved"]}

def submit(task_id, token, evidence):
    with locked() as d:
        t, err = _auth(d, task_id, token)
        if err: return err
        t.update(state="submitted", evidence=evidence)
        return {"task_id": task_id, "state": "submitted"}

def post(to, frm, text):
    with locked() as d:
        mid = d["next_msg"]; d["next_msg"] += 1
        d["inbox"].setdefault(to, []).append({"id": mid, "from": frm, "text": text, "acked": False, "at": time.time()})
        return {"id": mid}

def inbox(member):
    return {"messages": [m for m in read()["inbox"].get(member, []) if not m["acked"]]}

def send(frm, to, text):
    with locked() as d:
        d["outbox"].append({"from": frm, "to": to, "text": text, "at": time.time()})
    if to != "coordinator":
        return post(to, frm, text)
    return {"sent": True}

def ack(member, mid):
    with locked() as d:
        for m in d["inbox"].get(member, []):
            if m["id"] == mid:
                m["acked"] = True; return {"acked": mid}
        return {"error": f"no message {mid}"}

def reserved_files(member):
    return sorted({f for t in read()["tasks"].values() if t["claimant"] == member for f in t["reserved"]})
