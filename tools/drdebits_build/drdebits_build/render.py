"""Markdown rendering primitives. Explicit line construction only."""
from __future__ import annotations


def render_frontmatter(meta):
    lines = ["---"]
    lines += [f"{k}: {v}" for k, v in meta.items()]
    lines.append("---")
    return "\n".join(lines) + "\n"


def render_table(headers, aligns, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(aligns) + "|"]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out) + "\n"
