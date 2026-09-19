# GNOME Window Tracking

## Current Status

WayExpand does not currently support window tracking (app-filtering) on GNOME. This means `app_filter` entries in your configuration are ignored on GNOME, and expansions will match globally rather than per-application.

## Why GNOME is Not Supported

Window tracking on Wayland requires compositor cooperation. Most desktop environments provide this through:

1. **D-Bus `org.freedesktop.DBus.Properties.GetAll()` on a window interface** (KDE Plasma, GNOME <= 3.36)
2. **Wayland text-input protocol metadata** (experimental, not widely adopted)
3. **Direct protocol support** (wlroots compositors)

GNOME removed its D-Bus window interface in GNOME 4.0 (released 2021) and has not provided an alternative path for third-party applications to query the focused window. This makes it impossible for WayExpand to know which application has keyboard focus.

## Workarounds

### Option 1: Use global expansions (recommended for most users)

Remove `app_filter` entries from your configuration and use globally-scoped expansions:

```toml
[[expansion]]
trigger = ":email"
replacement = "user@example.com"
# No app_filter = expansion is global
```

### Option 2: Use GNOME Extensions (experimental)

If you need per-app expansion control, consider writing a GNOME Shell extension that:
- Monitors the active window
- Communicates with WayExpand via a socket API
- Enables/disables expansions dynamically

This would require changes to WayExpand to support external control of matching. Open an issue if you're interested in this approach.

### Option 3: Run KDE Plasma (full support)

If per-app expansion is critical, KDE Plasma (6.6+) provides full window tracking support. WayExpand works best on KDE Plasma with all features enabled.

### Option 4: Use input-method-v2 without app filtering

WayExpand's input-method-v2 backend (the default on GNOME) provides:
- Automatic sensitive-field detection (password inputs are excluded)
- Per-content-type behavior (passwords, hidden text, etc. are handled separately)

This covers the primary security use case for app filtering, even without per-app control.

## How to Check Your Environment

```bash
wayexpand doctor
```

The output will show:
- **`app-id tracking`**: The status of window tracking support
  - `Implemented` (KDE Plasma with D-Bus, or wlroots compositors)
  - `NotImplemented` (GNOME, Cosmic, other Wayland compositors without D-Bus)
  - `RequiresPermission` (available but needs portal consent)

## Contributing Support for GNOME

If you'd like to help add GNOME support:

1. **Monitor upcoming GNOME APIs**: GNOME's accessibility API or a future Wayland protocol extension might provide window information. Subscribe to [GNOME Shell development](https://gitlab.gnome.org/GNOME/gnome-shell/-/issues) for updates.

2. **Implement a custom D-Bus service**: A separate GNOME extension could provide a simple D-Bus interface that WayExpand queries for the focused app. This would be a community contribution.

3. **Support layering in WayExpand**: Propose an architecture where external tools (like a GNOME extension) can query or control expansion state via a socket API.

## See Also

- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — Compositor feature matrix
- [SECURITY.md](../SECURITY.md) — Sensitive field handling
- [docs/wiki/Configuration.md](wiki/Configuration.md#app-filtering) — app_filter configuration reference
