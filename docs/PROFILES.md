# Local profiles and application defaults

Profiles are independent, validated snippet libraries stored under the
configuration directory's `profiles/` directory. They work offline and do not
require a hosted account.

```sh
wayexpand profile create personal
wayexpand profile create work
wayexpand profile list
wayexpand profile switch work
```

`switch` activates a profile as the primary configuration. The current active
library is checkpointed back to its profile before switching.

## Application defaults

Use an exact desktop application ID to select a profile automatically:

```sh
wayexpand profile assign-app org.kde.konsole work
wayexpand profile unassign-app org.kde.konsole
wayexpand profile list
```

The GUI's **More actions → Profiles** menu also lists current assignments and
lets you add or remove them. It displays the runtime-selected profile when an
application assignment is active.

The daemon uses the focused window's application ID. Assignments are exact and
case-insensitive; wildcard patterns and window titles are not used. When no
assignment matches, WayExpand returns to the base configuration. The mapping
is saved in `profiles/applications.toml` with private file permissions.

When focus changes to an assigned application, matching pauses while its
profile is loaded and validated. If loading fails, matching remains paused
rather than silently using another application's snippets. Correct the profile
or assignment, then run `wayexpand reload` (or restart the daemon) to retry.
The GUI shows the runtime-selected profile and distinguishes an in-progress
load from a completed validation failure.

Application IDs vary by compositor and application packaging. The assignment
matches the ID reported by the active window tracker; if the tracker cannot
provide an ID, no application assignment is selected.
