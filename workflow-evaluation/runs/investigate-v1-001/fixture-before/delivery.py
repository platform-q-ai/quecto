def deliver(send, receipt):
    for attempt in range(2):
        try:
            return send(receipt)
        except TimeoutError:
            if attempt == 1:
                raise
