def total(values):
    answer = 0
    for value in values:
        answer += value
    return answer


def mean(values):
    if not values:
        return 0
    return total(values) / len(values)
