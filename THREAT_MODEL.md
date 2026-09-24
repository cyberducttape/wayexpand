# WayExpand threat model

WayExpand is a local desktop text expander. It is designed to run as the
unprivileged desktop user, stay offline, avoid telemetry, and keep keyboard
capture, configuration, command execution, and control interfaces bounded.

This document is the short security map. For deeper details, see
[SECURITY.md](SECURITY.md), [docs/BACKENDS.md](docs/BACKENDS.md),
[docs/BACKENDS_SENSITIVE_FIELDS.md](docs/BACKENDS_SENSITIVE_FIELDS.md), and
[docs/EVDEV_ACCESS_DESIGN.md](docs/EVDEV_ACCESS_DESIGN.md).

## Assumptions

- WayExpand is not run as root.
- The user's account, home directory, and graphical session are trusted at the
  same level as other same-user desktop applications.
- Snippet configuration is trusted user content. Command-backed snippets are
  executable user content.
- Root, kernel, compositor, and package-manager compromise are outside the
  boundary WayExpand can defend.

## Summary table

| Threat | Protected? | Notes |
|--------|------------|-------|
| Telemetry or cloud upload | Yes | WayExpand has no account, cloud service, or telemetry path. |
| Raw input logged by WayExpand | Yes | Normal logs record counts and state, not typed text, trigger names, or replacement contents. |
| Malicious config replacement | Yes | Config files must be regular files, owned by the user or root, and not group/world-writable. Parent directories are validated too. |
| Symlink config attack | Yes | Config paths are resolved and validated before use; atomic saves use safe permissions and descriptor checks. |
| Unsafe config reload | Yes | Reloads parse and validate before replacing the live configuration. Bad reloads fail closed. |
| Malicious trigger shell injection | Yes | Command snippets execute a program and argument vector directly; WayExpand does not use `sh -c`. |
| Unbounded command output | Yes | Command stdout is UTF-8 checked and capped; stderr is discarded; stdin is null. |
| Slow or stuck command | Mostly | Direct commands have timeouts and process-group cleanup. This is resource cleanup, not service/cgroup containment. |
| Command snippet side effects | Partially | Commands run as the desktop user under the hardened systemd unit. Treat command-enabled config as executable content. The planned Action Broker is the stronger boundary. |
| PATH surprises in command snippets | Partially | Organization policy can require absolute command paths. Without that policy, relative program names resolve through the daemon's environment. |
| Another local user controlling the daemon | Yes | The control socket is created under a validated runtime path, mode `0600`, and stale-socket cleanup checks owner and identity. |
| Same-user process controlling or inspecting desktop state | Out of scope | A process with the same UID can generally inspect or interfere with the user's desktop session. Use OS sandboxing for mutually untrusted same-user apps. |
| Password capture under input-method/IBus-style sources | Backend-dependent | Sources that receive sensitive-field signals must start disabled, clear matcher state, and suspend matching while sensitive content is focused. |
| Password capture under evdev | No | Evdev reads kernel input events and has no password-field signal. This is an intrinsic backend limitation. |
| Evdev permission granted during normal install | Yes | Base installers/packages do not activate raw-input policy. Evdev access requires an explicit root step. |
| Evdev permission broader than keyboard matching | Partially | WayExpand's udev templates target keyboard-class event nodes; legacy `input` group policy may still be broader on some distributions. |
| Wrong-window insertion | Mostly | Backend/window tracking and generation checks reduce stale insertion risk. Certification still depends on compositor/application combinations. |
| Root compromise | Out of scope | Root controls input devices, configuration locations, service units, and binaries. |
| Compositor/protocol compromise | Out of scope | WayExpand depends on compositor and input protocol correctness for focus, sensitive-field, and injection semantics. |
| Portal token path manipulation | Yes | Portal token paths use trusted-parent validation and reject group/world-writable non-sticky ancestors. |
| Systemd service escape by WayExpand | Defense in depth | User units enable restrictive filesystem, process, memory, namespace, and privilege settings. |

## Backend-specific security boundary

The backend choice determines what WayExpand can know about focused fields and
what permissions it needs.

| Backend/source | Security property | Main limitation |
|----------------|-------------------|-----------------|
| IBus / input-method-style sources | Can receive field sensitivity and suspend matching in password fields when the protocol reports it. | Compatibility and non-text key handling depend on desktop/protocol behavior. |
| evdev | Preserves physical key visibility across compositors after explicit raw-input opt-in. | No password-field signal, no per-app content semantics, and non-exclusive capture cannot be strictly atomic. |
| libei / portal output | Uses compositor-mediated injection/portal consent where available. | Consent and portal behavior are compositor-dependent; output does not by itself solve capture privacy. |
| wlroots virtual keyboard output | Native output path for wlroots compositors that expose the protocol. | Compositor-specific and not a universal desktop boundary. |

## Command execution boundary

Direct command snippets are intentionally safer than shell filters, but they are
not a full automation broker:

- no shell interpretation;
- bounded arguments, runtime, stdout size, and UTF-8 output;
- no stdin and discarded stderr;
- hardened user service environment;
- optional organization policy requiring absolute command paths.

They still execute a trusted program as the desktop user. Commands that need
network access, cloud credentials, broad filesystem writes, or auditable
approval should wait for the planned Action Broker described in
[docs/ACTION_BROKER_DESIGN.md](docs/ACTION_BROKER_DESIGN.md).

## Practical deployment guidance

- Prefer compositor/input-method paths that provide sensitive-field signals.
- Treat evdev as an explicit compatibility fallback, not a transparent default.
- Use `safe_mode = true` or `disable_commands = true` for fleets that do not
  need command-backed snippets.
- Use `require_absolute_commands = true` for managed deployments that allow
  commands.
- Keep the shipped systemd hardening unless you are deliberately changing the
  trust boundary.
- Revoke evdev access explicitly when retiring an evdev deployment:
  `sudo wayexpand-install-evdev-access --uninstall` or
  `sudo ./scripts/install-evdev-permissions.sh --uninstall`.
