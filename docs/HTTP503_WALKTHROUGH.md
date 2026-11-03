# HTTP 503 first-response walkthrough

This walkthrough demonstrates WayExpand's first sysadmin workflow: turn an
HTTP 503 incident into a validated, reviewable checklist for the host and
service the operator selects. It generates text only. WayExpand never runs
these diagnostic commands.

## First run

Install or build WayExpand using the current
[`Getting Started`](GETTING_STARTED.md) instructions, then start setup and
inspect the route reported for this desktop session:

```sh
wayexpand setup
wayexpand status --json
wayexpand doctor --certify --json
```

`connected` means a route is available; it does not mean that route is
production-certified. Check the certification report and the current
[support matrix](SUPPORT_MATRIX.md). The checklist generation and validation
below run locally through the CLI and do not require keyboard injection.

Install the reviewed operations pack from a WayExpand source checkout:

```sh
wayexpand pack install ./packs/ops-triage
wayexpand pack lint ./packs/ops-triage --json
wayexpand pack test ./packs/ops-triage --json
```

If you already have an Espanso or AutoKey library, import it through the
documented [migration workflow](MIGRATION_FROM_ESPANSO.md) or
[AutoKey importer](MIGRATION_FROM_AUTOKEY.md); importing remains optional.

## Generate and review the checklist

Generate the checklist as a private staged file. The form asks for environment,
local or remote SSH session, expected host, service, proxy unit, and port. The
field validators reject malformed host names, unit names, and ports. The
registered WayExpand validator checks that the generated shell blocks contain
only the read-only diagnostic command set. Review the destination and diff,
then type `write` when prompted to publish the file:

```sh
wayexpand workflow generate http503 \
  --set 'environment=staging' \
  --set 'session context=remote SSH' \
  --set 'target host=web-01.example.net' \
  --set 'service=nginx.service' \
  --set 'proxy service=nginx.service' \
  --set 'port=8080' \
  -o ./incident-http503.md
```

The session selection is an operator declaration, not detected desktop or
terminal state. Before running a command, verify that the current shell is the
selected local or SSH session and that `hostname -f` matches the expected
host. WayExpand does not inspect terminal text, shell history, SSH state, or
credentials; it does not initiate an SSH connection.

## Copy one command

The `;http503-service`, `;http503-journal`, `;http503-sockets`, and
`;http503-proxy` forms generate one diagnostic command at a time. Generate and
inspect the exact command first:

```sh
wayexpand new ';http503-service' \
  --set 'session context=remote SSH' \
  --set 'target host=web-01.example.net' \
  --set 'service=nginx.service' \
  --stdout
```

After reviewing the output and confirming the target, run the same generator
with a filter to copy only the command line. This writes to the clipboard; it
does not run the command:

```sh
wayexpand new ';http503-service' \
  --set 'session context=remote SSH' \
  --set 'target host=web-01.example.net' \
  --set 'service=nginx.service' \
  --stdout \
  | sed -n '/^systemctl status /p' | wl-copy
```

Use the matching `sed` prefix for another command form. This writes the
generated command to the desktop clipboard only; it does not run it. Review
the clipboard contents and terminal target before pasting. If `wl-copy` is
unavailable, print the selected command and copy it using the desktop.

The main `;http503` expansion inserts the complete checklist on a route that
supports verified forms. Some desktop routes intentionally refuse form-based
replacement until they meet the project's target-integrity requirements; use
the CLI Generate File or command-copy flow on those sessions.

The checklist runs `hostname -f`, `systemctl status`, bounded `journalctl`,
`ss -lntp`, a bounded localhost `curl -I` request with the selected Host
header, and `nginx -t`. It does not restart a service, alter configuration,
press Enter, or infer which machine the current shell controls. Redact
credentials and customer data before sharing diagnostic output.
