import html
def text_receipt(items):
    lines = []
    for name, qty, cents in items:
        unit = "item" if qty == 1 else "items"
        line = f"{name}: {qty} {unit} @ {cents}c"
        lines.append(line)
    return "\n".join(lines)

def html_receipt(items):
    lines = []
    for name, qty, cents in items:
        unit = "item" if qty == 1 else "items"
        line = f"{name}: {qty} {unit} @ {cents}c"
        lines.append("<li>" + html.escape(line) + "</li>")
    return "<ul>" + "".join(lines) + "</ul>"
