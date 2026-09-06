def merge(intervals):
    result = []
    for start, end in sorted(intervals):
        if result and start <= result[-1][1]:
            left, right = result[-1]
            result[-1] = (left, max(right, end))
        else:
            result.append((start, end))
    return result
