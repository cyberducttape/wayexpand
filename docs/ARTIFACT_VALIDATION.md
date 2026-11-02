# Explicit artifact validation

`wayexpand new NAME --validate` validates a generated infrastructure artifact
before publishing it. Validation is opt-in, uses a fixed registry in the CLI,
and never takes an executable or shell fragment from a snippet or imported
pack. A template with no registered validator is rejected without writing an
output file.

For example:

```sh
wayexpand new playbook \
  --set hosts=webservers \
  --set package=nginx \
  --validate
```

This runs `ansible-playbook --syntax-check`; it does not run the playbook or
apply infrastructure changes. Generated files keep the normal no-overwrite
default. `--force` only replaces the destination after validation succeeds.
Ansible role generation stages and validates the complete role before its
atomic directory publish.

## Registered validators

| Generated artifact | Required executable | Invocation |
| --- | --- | --- |
| Ansible playbook and role | `ansible-playbook` | `--syntax-check` |
| Dockerfile variants | `hadolint` | lint generated file |
| Compose file | `docker` | `compose config --quiet` |
| systemd service | `systemd-analyze` | `verify` generated unit |
| GitHub Actions workflow | `actionlint` | lint generated workflow |
| Nginx site | `nginx` | `-t` with an isolated wrapper config |
| Kubernetes deployment | `kubeconform` | `-strict -summary` |

The selected validator must be installed separately. Missing tools produce a
clear startup error. Every validator runs with a 30-second deadline in its own
process group and receives a cleared environment, a private temporary HOME,
and no inherited credential variables. Kubeconform may fetch schemas using
its normal schema sources, so `--validate` for Kubernetes manifests requires
network access unless kubeconform has a usable local cache.

This boundary is intentionally narrow: validation is an explicit local
operation and invokes only WayExpand's fixed executable and arguments. The
validator still runs as the invoking user; this is not an OS sandbox. Review
generated infrastructure before applying it, and use the target tool's
separate dry-run or policy controls where appropriate. WayExpand does not
execute generated commands during passive keyboard capture, and templates
cannot declare validators or interpolate secrets through this path.

Validation happens on a private staging copy before the destination is
published. A failed validator leaves an existing destination unchanged.
Template-defined output paths used during staging must be relative and may
not escape the temporary workspace.
