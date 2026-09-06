def resolve(defaults, overrides):
    return {key: overrides.get(key) or value for key, value in defaults.items()}
