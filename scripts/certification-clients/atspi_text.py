#!/usr/bin/env python3
"""Read a real application's text widget over AT-SPI.

    atspi_text.py APP_NAME STATE_FILE

Finds the accessible application APP_NAME on the private session's
accessibility bus, picks its focused editable text object (or, when none is
focused, its first multi-line editable one), and writes STATE_FILE as JSON:
{"app_id", "focused", "text", "cursor"}. Exits 1 when no such object exists.
Only run inside the private certification session.
"""

import json
import os
import sys

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402


def candidates(node, depth=0):
    if depth > 30:
        return
    try:
        states = node.get_state_set()
        if node.get_text_iface() is not None and states.contains(Atspi.StateType.EDITABLE):
            yield node, states
        for index in range(node.get_child_count()):
            child = node.get_child_at_index(index)
            if child is not None:
                yield from candidates(child, depth + 1)
    except gi.repository.GLib.Error:
        return


def main():
    app_name, state_file = sys.argv[1], sys.argv[2]
    desktop = Atspi.get_desktop(0)
    apps = [desktop.get_child_at_index(i) for i in range(desktop.get_child_count())]
    app = next((a for a in apps if a is not None and a.get_name() == app_name), None)
    if app is None:
        return 1
    found = list(candidates(app))
    chosen = next((pair for pair in found if pair[1].contains(Atspi.StateType.FOCUSED)), None)
    if chosen is None:
        chosen = next(
            (pair for pair in found if pair[1].contains(Atspi.StateType.MULTI_LINE)), None
        )
    if chosen is None:
        return 1
    node, states = chosen
    temporary = state_file + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        json.dump(
            {
                "app_id": app_name,
                "focused": states.contains(Atspi.StateType.FOCUSED),
                "text": Atspi.Text.get_text(node, 0, -1),
                "cursor": Atspi.Text.get_caret_offset(node),
            },
            handle,
        )
    os.replace(temporary, state_file)
    return 0


if __name__ == "__main__":
    sys.exit(main())
