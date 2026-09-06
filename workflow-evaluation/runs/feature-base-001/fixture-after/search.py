def find(names, query, *, limit=None):
    """Return case-insensitive substring matches in input order, up to limit."""
    if limit is not None and limit < 0:
        raise ValueError("limit must be nonnegative")
    if limit == 0:
        return []

    needle = query.casefold()
    matches = []
    for name in names:
        if needle in name.casefold():
            matches.append(name)
            if limit is not None and len(matches) >= limit:
                break
    return matches
