#!/usr/bin/env python3
"""GTK 3 client for the KWin certification driver.

    gtk_client.py APP_ID STATE_FILE [--password]

Shows one real GTK text widget (a multi-line TextView, or with --password a
password-purpose Entry) and records what the toolkit delivered to it in
STATE_FILE as JSON: {"app_id", "focused", "text", "cursor"}. The cursor is a
character offset. Only run inside the private certification session.
"""

import json
import os
import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk  # noqa: E402


def main():
    app_id, state_file = sys.argv[1], sys.argv[2]
    password = "--password" in sys.argv[3:]
    GLib.set_prgname(app_id)
    window = Gtk.Window(title=app_id)
    window.set_default_size(640, 400)
    if password:
        widget = Gtk.Entry()
        widget.set_visibility(False)
        widget.set_input_purpose(Gtk.InputPurpose.PASSWORD)
    else:
        widget = Gtk.TextView()
    window.add(widget)

    def state():
        if password:
            return widget.get_text(), widget.get_position()
        buffer = widget.get_buffer()
        text = buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)
        return text, buffer.get_iter_at_mark(buffer.get_insert()).get_offset()

    def write(*_):
        text, cursor = state()
        temporary = state_file + ".tmp"
        with open(temporary, "w", encoding="utf-8") as handle:
            json.dump(
                {"app_id": app_id, "focused": window.is_active(), "text": text, "cursor": cursor},
                handle,
            )
        os.replace(temporary, state_file)

    if password:
        widget.connect("changed", write)
        widget.connect("notify::cursor-position", write)
    else:
        widget.get_buffer().connect("changed", write)
        widget.get_buffer().connect("mark-set", write)
    window.connect("notify::is-active", write)
    window.connect("destroy", Gtk.main_quit)
    window.show_all()
    widget.grab_focus()
    write()
    Gtk.main()


if __name__ == "__main__":
    main()
