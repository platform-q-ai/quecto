from classification import classify_name as _classify_name


def summarize(names):
    result = {}
    for name in names:
        category = _classify_name(name)
        result[category] = result.get(category, 0) + 1
    return result
