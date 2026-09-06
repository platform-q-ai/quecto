def decide(active, locked, admin, owner):
    if not active:
        return False, "inactive"
    if locked:
        return False, "locked"
    if admin:
        return True, "admin"
    if owner:
        return True, "owner"
    return False, "not-owner"
