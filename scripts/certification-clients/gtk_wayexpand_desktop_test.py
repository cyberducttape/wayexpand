"""Content-free readback client used by `wayexpand debug --test --controlled-client`.

It opens one GTK 3 entry in the active desktop session and reads only whether
the complete entry equals the configured harmless test replacement. It never
prints or persists field contents. The user types the trigger; the helper does
not synthesize input or press Enter.
"""

import json
import sys


def report(state, key_events=0):
    print(json.dumps({"state": state, "key_events": key_events}, separators=(",", ":")))


def main():
    try:
        import gi

        gi.require_version("Gtk", "3.0")
        from gi.repository import GLib, Gtk
    except (ImportError, ValueError):
        report("unavailable")
        return 77

    try:
        contract = json.load(sys.stdin)
        trigger = contract["trigger"]
        input_marker = contract["input_marker"]
        expected = contract["expected"]
        if (
            not isinstance(trigger, str)
            or not isinstance(input_marker, str)
            or not isinstance(expected, str)
        ):
            report("invalid_contract")
            return 2
        if (
            not trigger
            or not input_marker
            or not expected
            or trigger == expected
            or input_marker == expected
        ):
            report("invalid_contract")
            return 2
        if "\n" in expected or "\r" in expected:
            report("unavailable")
            return 77
    except (KeyError, TypeError, ValueError):
        report("invalid_contract")
        return 2

    Gtk.init([])
    window = Gtk.Window(title="WayExpand desktop test")
    window.set_default_size(520, 190)
    window.set_position(Gtk.WindowPosition.CENTER)
    window.set_border_width(18)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
    prompt = Gtk.Label(label=f"Type this trigger once: {trigger}")
    prompt.set_xalign(0)
    entry = Gtk.Entry()
    entry.set_activates_default(False)
    entry.set_placeholder_text("Click here, then type the trigger")
    explanation = Gtk.Label(
        label="WayExpand will check this field directly. No Enter key is sent."
    )
    explanation.set_xalign(0)
    box.pack_start(prompt, False, False, 0)
    box.pack_start(entry, False, False, 0)
    box.pack_start(explanation, False, False, 0)
    window.add(box)

    key_events = 0
    input_marker_observed = False
    finished = False
    started = GLib.get_monotonic_time()

    def count_key(_widget, _event):
        nonlocal key_events
        key_events += 1
        return False

    def observe_input_marker(_widget):
        nonlocal input_marker_observed
        # The field starts empty. Seeing the expected typed input in this
        # field before its replacement rules out a pasted expected result as
        # evidence that the desktop integration worked. Immediate IBus matches
        # consume the final key, so input_marker may be the trigger prefix that
        # the client actually receives.
        if entry.get_text() == input_marker:
            input_marker_observed = True

    entry.connect("key-press-event", count_key)
    entry.connect("changed", observe_input_marker)

    def finish(state):
        nonlocal finished
        if finished:
            return
        finished = True
        report(state, key_events)
        Gtk.main_quit()

    def inspect_field():
        value = entry.get_text()
        if value == expected and key_events > 0 and input_marker_observed:
            finish("verified")
            return GLib.SOURCE_REMOVE
        if GLib.get_monotonic_time() - started >= 45_000_000:
            if key_events == 0:
                finish("input_not_observed")
            elif value == expected and not input_marker_observed:
                finish("output_without_input_marker")
            elif value == trigger:
                finish("trigger_not_replaced")
            else:
                finish("field_mismatch")
            return GLib.SOURCE_REMOVE
        return GLib.SOURCE_CONTINUE

    window.connect("destroy", lambda *_: finish("closed"))
    window.show_all()
    window.present()
    entry.grab_focus()
    GLib.timeout_add(40, inspect_field)
    Gtk.main()
    return (
        0
        if finished
        and entry.get_text() == expected
        and key_events > 0
        and input_marker_observed
        else 1
    )


if __name__ == "__main__":
    raise SystemExit(main())
