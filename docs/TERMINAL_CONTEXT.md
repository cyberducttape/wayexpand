# Terminal and remote session context

WayExpand can report the context of an opted-in local shell session and use a
fresh report to render a context-aware text snippet. The report is not an
authorization claim. Generated commands remain preview text; `context render`
does not type, paste, or execute them.

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
waits for any in-flight refresh, then clears the remote report before publishing
local context after SSH exits. Use the wrapper for shell destinations that
need reporting. A direct `ssh`
invocation is intentionally left untouched and does not cause WayExpand to
guess the destination. The reported destination is the explicit SSH target or
alias, not a DNS-resolved host identity.

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
credentials or authorization claims. In particular, a Kubernetes context name
does not prove the active cluster's identity or safety.

## Render shell-aware snippets

Templates may use `{{context:shell}}`, `{{context:remote_host}}`,
`{{context:environment}}`, `{{context:project_root}}`,
`{{context:kubernetes_context}}`, and `{{context:container}}`. For values used
as shell arguments, use the POSIX quoting form, for example
`{{shellq:context:remote_host}}`. Context markers fail closed when there is no
fresh report or explicit context.

The reviewed `;;ssh-health` snippet demonstrates a validated multi-line,
read-only checklist. Render it from an opted-in shell session with a fresh
report, or supply the target explicitly before connecting:

```sh
wayexpand context render ';;ssh-health' \
  --shell bash \
  --remote-host web-staging \
  --environment staging \
  --set 'field:expected host=web-01.example.net' \
  --set 'field:service=api.service' \
  --set 'field:lines=75' \
  --set 'field:port=8080'
```

The Kubernetes operations pack uses the same fresh context report to pin
`kubectl` commands. If the shell integration is not installed, an operator can
select a context explicitly for a text-only preview:

```sh
wayexpand context render ';;kube-logs' --shell bash \
  --kubernetes-context staging-cluster \
  --set 'field:namespace=payments' \
  --set 'field:pod=api-7f8c9d6b4f-2kq' \
  --set 'field:container=api' \
  --set 'field:since=15m' \
  --set 'field:lines=200'
```

The preview labels the selected Kubernetes context and prints the exact
shell-quoted command. WayExpand does not invoke `kubectl` or run the command.
Every Kubernetes operations-pack command includes a validated context field;
the shell report can prefill it in a CLI preview. Ordinary desktop forms ask
the operator to choose the context. These commands never inherit the
workstation's `kubectl` default implicitly. The CLI marks the selected target
unverified: WayExpand does not check that the name exists in kubeconfig or
identify the cluster behind it, and does not contact Kubernetes.

The command prints the completed text to stdout and labels the selected shell,
environment, and SSH destination. It refuses command-backed snippets. It does
not insert text into a terminal buffer, run a command, or press Enter. Review
the host and each line; do not paste a whole multi-line block into a shell that
might execute pasted newlines. Copy only the individual command(s) you intend
to run after connecting to the selected target. Use `--json` to obtain a
machine-readable preview with `execution: "never"`.
