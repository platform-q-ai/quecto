def resolve(defaults, overrides):
    return {
        key: value if (override := overrides.get(key)) is None else override
        for key, value in defaults.items()
    }
