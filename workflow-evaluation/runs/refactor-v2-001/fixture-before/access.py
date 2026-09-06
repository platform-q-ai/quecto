def decide(active, locked, admin, owner):
    if active:
        if not locked:
            if admin:
                return True, "admin"
            else:
                if owner:
                    return True, "owner"
                else:
                    return False, "not-owner"
        else:
            return False, "locked"
    else:
        return False, "inactive"
