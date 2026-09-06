from delivery import deliver
calls = []
def send(receipt):
    calls.append(receipt)
    if len(calls) == 1:
        raise TimeoutError("ack lost")
    return "ok"
assert deliver(send, "R7") == "ok" and calls == ["R7", "R7"]
print("timeout triggers a second send of the same receipt; receiver side effects unknown")
