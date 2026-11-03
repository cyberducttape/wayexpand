# Safety-aware workflows

WayExpand exposes three explicit operator workflows. Text insertion, file
generation, and command execution are separate choices.

## Insert

Insert places the selected trigger's replacement in the focused application.
The command captures the exact current window and the daemon verifies that it
is still focused when insertion begins:

```sh
wayexpand workflow insert ';http503'
```

The daemon's route and target safety policy still applies. Insert does not run
the generated shell commands or press Enter.

## Generate File

Generate File renders a configured or built-in artifact into a private staging
area, runs the registered validator, shows the destination and a bounded diff,
and asks the operator to type `write` before it publishes anything:

```sh
wayexpand workflow generate playbook \
  --set hosts=webservers --set package=nginx -o deploy.yml
```

The validator is selected by WayExpand from its built-in registry. Imported
templates cannot supply executable validators or shell commands. Validators
run with a cleared environment and a deadline. A validation error or rejected
review leaves the requested destination untouched. Existing outputs require
`--force`; their current content is included in the review. Directory
scaffolds show each file's destination and proposed changes before one
confirmation writes the staged project.

The corresponding lower-level command remains available:

```sh
wayexpand new playbook --set hosts=webservers --set package=nginx \
  -o deploy.yml --validate --review
```

`--review` is interactive by design. For non-interactive automation, generate
to stdout or a private staging location, validate there, and use the automation
system's own separately reviewed publication step.

## Run Approved Action

Run Approved Action accepts only the ID of an enabled snippet configured as a
named Action Broker action. It displays the action identity and purpose, then
requires the operator to type its exact action approval phrase. A noninteractive
caller must pass `--approve-action ACTION_ID`, which must exactly match the
requested ID. User values are passed as structured parameters and checked by
the broker's action schema; values are not printed by the CLI.

```sh
wayexpand workflow run collect-service-status \
  --set unit=nginx.service
```

The broker independently checks the caller, action allowlist, executable,
arguments, parameter schema, sandbox and deadline. The workflow refuses to run
unless broker health reports audit enabled, mandatory and healthy. The broker
persists its completion record before returning success, so the final CLI
result includes the action outcome, exit code, duration and audit confirmation.
The configured broker audit record excludes parameter values. Network access
remains restricted by the broker's own service policy.

Review the active policy and audit state with:

```sh
wayexpand doctor --json
```

The workflow does not assume the local workstation is the target environment.
An action's destination and privileges are properties of its reviewed broker
definition and policy; specify remote or cluster context only through
parameters accepted by that definition. Text generation remains separate from
execution, and infrastructure-changing actions require the explicit action
approval each time.
