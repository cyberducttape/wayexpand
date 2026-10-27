# Checking snippet repositories in CI

**Navigation:** [Home](../README.md) > [System Administration](FOR_SYSADMINS.md) > **Snippet repositories**

---

Teams that keep snippets in Git can check every change before it reaches
anyone's desktop. `scripts/check-snippet-repo.sh` runs the same checks the
daemon would apply, plus a trigger-risk review:

- every TOML file with `[[expansion]]` or `[[hotkey]]` entries must load and
  validate (other TOML files, such as `Cargo.toml`, are skipped);
- every directory with a `wayexpand-pack.toml` must be a valid
  [pack](PACKS.md), and with `--signers FILE` its signature must verify;
- triggers likely to fire by accident are reported: ordinary words in
  immediate mode, very short triggers, and triggers that are the start of
  another. They are warnings unless `--deny-risks` makes them failures.

Output names files and triggers but never replacement text. Under GitHub
Actions each finding is also an annotation on the file.

## GitHub Actions

```yaml
name: Snippets
on: [pull_request, push]
permissions:
  contents: read
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: cyberducttape/wayexpand@main  # pin a release tag or commit SHA
        with:
          paths: snippets packs
          deny-risks: true
          signers: .wayexpand/allowed_signers
```

The action builds the CLI from its own revision, so pinning the action pins
the validator too. The first run compiles WayExpand; later runs reuse the
Cargo cache.

## pre-commit

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/cyberducttape/wayexpand
    rev: main  # pin a release tag or commit SHA
    hooks:
      - id: wayexpand-snippets          # risks are warnings
      # - id: wayexpand-snippets-strict # risks fail the commit
```

The hooks run the installed `wayexpand` (or `WAYEXPAND_CLI`) and need `jq`.

## Locally or in other CI systems

```sh
scripts/check-snippet-repo.sh                      # the whole repository
scripts/check-snippet-repo.sh --deny-risks team/   # one directory, strict
scripts/check-snippet-repo.sh --signers allowed_signers packs/
```

Exit status is 0 when every check passed, 1 when any failed, and 2 for usage
errors. The same risk review is available for a single file with
`wayexpand validate --json [--deny-risks] FILE`, whose report includes
`trigger_risks` and `command_count`.

`wayexpand validate` refuses configuration files that other users can read,
as every checkout is. The script therefore validates a private copy of the
repository's TOML files; the repository itself is never modified.
