import html


def _format_line(name, qty, cents):
    unit = "item" if qty == 1 else "items"
    return f"{name}: {qty} {unit} @ {cents}c"


def text_receipt(items):
    lines = []
    for name, qty, cents in items:
        line = _format_line(name, qty, cents)
        lines.append(line)
    return "\n".join(lines)


def html_receipt(items):
    lines = []
    for name, qty, cents in items:
        line = _format_line(name, qty, cents)
        lines.append("<li>" + html.escape(line) + "</li>")
    return "<ul>" + "".join(lines) + "</ul>"
