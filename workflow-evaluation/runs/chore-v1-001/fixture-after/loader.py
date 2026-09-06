def validate(config):
    extra = set(config) - {"host", "port", "debug", "logging"}
    if extra:
        raise ValueError("unknown fields: " + ",".join(sorted(extra)))
    return dict(config)
