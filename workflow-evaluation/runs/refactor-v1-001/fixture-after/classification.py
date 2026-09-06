"""Internal filename classification rules; not a public API."""


def classify_name(name):
    suffix = name.rsplit(".", 1)[-1] if "." in name else ""
    if suffix in ("txt", "md"):
        return "text"
    elif suffix in ("csv", "json"):
        return "data"
    else:
        return "other"
