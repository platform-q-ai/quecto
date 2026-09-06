def total_cents(amounts):
    """Return the exact total in cents for plain decimal currency strings."""
    total = 0
    for amount in amounts:
        whole, _, fraction = amount.partition(".")
        total += int(whole) * 100 + int(fraction.ljust(2, "0"))
    return total
