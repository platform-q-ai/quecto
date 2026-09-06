def summarize(names):
    result = {}
    for name in names:
        suffix = name.rsplit(".", 1)[-1] if "." in name else ""
        if suffix in ("txt", "md"):
            category = "text"
        elif suffix in ("csv", "json"):
            category = "data"
        else:
            category = "other"
        result[category] = result.get(category, 0) + 1
    return result
