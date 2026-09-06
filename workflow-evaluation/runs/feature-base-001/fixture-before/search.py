def find(names, query):
    needle = query.casefold()
    return [name for name in names if needle in name.casefold()]
