#!/usr/bin/env python3
"""Ticks parity checklist items in migration/features/*.md once a track's
branch merges with them done.

    python3 migration/tick.py HOST-132..HOST-219 SESS-001 SESS-004..SESS-010

Each argument is an ID or an inclusive range of IDs with the same prefix.
Prints how many it ticked, and any ID it couldn't find, and exits non-zero
if one is missing, so a typo never passes silently.
"""

import pathlib
import re
import sys

FEATURES = pathlib.Path(__file__).resolve().parent / "features"


def expand(arg):
    if ".." not in arg:
        return [arg]
    start, end = arg.split("..")
    prefix, first = re.fullmatch(r"([A-Z]+)-(\d+)", start).groups()
    end_prefix, last = re.fullmatch(r"([A-Z]+)-(\d+)", end).groups()
    if prefix != end_prefix:
        sys.exit(f"tick: {arg} spans two prefixes")
    width = len(first)
    return [f"{prefix}-{n:0{width}d}" for n in range(int(first), int(last) + 1)]


def main():
    ids = [id_ for arg in sys.argv[1:] for id_ in expand(arg)]
    if not ids:
        sys.exit(__doc__)
    pending = set(ids)
    ticked = 0
    for path in sorted(FEATURES.glob("*.md")):
        text = path.read_text()
        lines = text.split("\n")
        for i, line in enumerate(lines):
            match = re.match(r"- \[( |x)\] ([A-Z]+-\d+)\b", line)
            if match and match.group(2) in pending:
                pending.discard(match.group(2))
                if match.group(1) == " ":
                    lines[i] = "- [x]" + line[5:]
                    ticked += 1
        new = "\n".join(lines)
        if new != text:
            path.write_text(new)
    print(f"tick: {ticked} ticked, {len(ids) - len(pending) - ticked} already ticked")
    if pending:
        sys.exit(f"tick: not found: {', '.join(sorted(pending))}")


if __name__ == "__main__":
    main()
