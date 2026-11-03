# Terminal and remote session context

WayExpand can report the context of an opted-in local shell session. The report
is descriptive metadata for operator visibility. It does not authorize an
action, choose a production target, or execute commands. Generated commands
remain reviewable text; any execution workflow must have its own explicit
authorization and confirmation.

## Enable a shell integration

Print the integration for the shell you use, review it, then source the reviewed
file from your shell startup configuration:

```sh
wayexpand context integration bash > ~/.config/wayexpand/context.bash
# Review the file, then add this line to ~/.bashrc:
source ~/.config/wayexpand/context.bash
```

Use `zsh` or `fish` in place of `bash` for those shells. The integration creates
a random session ID and reports at each prompt. Reports expire after 90 seconds
and are bound to the local controlling terminal device and a random shell
session ID. They do not depend on window focus, which can change while an SSH
session remains open. If WayExpand cannot identify the controlling terminal,
the report is refused. `wayexpand context status --json` displays the fresh
report for the current terminal and shell session; `wayexpand context
clear` removes it. Use `wayexpand help` for the command overview.

The integration obtains the Git root using `git rev-parse --show-toplevel` and
the selected Kubernetes context using `kubectl config current-context`. These
commands report the current local shell configuration. WayExpand does not
inspect shell history, terminal output, typed commands, or the contents of
terminal scrollback. Set `WAYEXPAND_CONTEXT_ENVIRONMENT` and
`WAYEXPAND_CONTEXT_CONTAINER` yourself when those labels are known; the
integration does not infer them from process names or prompt text.

## SSH context

The supplied integration defines an explicit `wayexpand-ssh` wrapper. The
destination must be its first argument:

```sh
wayexpand-ssh db-staging
wayexpand-ssh db-staging -p 2222 -- 'exec bash -l'
```

While that SSH process is active, the local terminal's context report includes
the literal destination supplied to the wrapper. It refreshes periodically and
returns to local context after SSH exits. Use the wrapper for shell destinations
that need reporting. A direct `ssh` invocation is intentionally left untouched
and does not cause WayExpand to guess the destination. The reported destination
is the explicit SSH target or alias, not a DNS-resolved host identity.

For a nested SSH session started inside a remote shell, the local integration
cannot observe that remote shell's process context. If nested-session
identification is needed, install an integration on that remote host and
configure a trusted label there; do not treat the local report as proof of the
remote command's execution environment.

## Manual reports

You can report a known context directly, with the session ID created by the
integration:

```sh
wayexpand context report \
  --environment staging \
  --remote-host db-staging \
  --project-root "$PWD" \
  --kubernetes-context staging-cluster \
  --container api-dev \
  --ttl 90
wayexpand context status
```

Context values are labels reported by the local shell integration, not verified
credentials or authorization claims.
In particular, a Kubernetes context name does not prove the active cluster's
identity or safety. Review the destination and generated command before use.
