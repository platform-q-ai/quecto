def total_cents(amounts):
    return sum(int(float(amount) * 100) for amount in amounts)
