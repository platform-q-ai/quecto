def total(rows):
    return sum(row["amount"] for row in rows)


def by_category(rows):
    """Sum amounts by category, preserving categories' first appearance."""
    result = {}
    for row in rows:
        category = row.get("category", "uncategorized")
        result[category] = result.get(category, 0) + row["amount"]
    return result
