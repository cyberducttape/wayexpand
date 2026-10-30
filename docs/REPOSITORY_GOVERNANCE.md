# Repository Governance

This document records the GitHub controls required to keep unverified code
from reaching `main` and to keep release promotion behind its existing
certification gate.

## Observed GitHub configuration

On 2026-10-30, the GitHub branch-protection API returned `Branch not
protected` for `main`, and the only repository ruleset (`pro`) was disabled. The current `main` CI run passed, but without an enforced
ruleset a contributor with write access can still merge or push changes while
required checks are failing or have not run. No online self-hosted
certification runners were registered at the time of the check.

The repository's release workflow already runs CI and the production
compositor certification workflow before producing release artifacts. That
tag-time gate does not protect ordinary changes merged into `main`.

## Required `main` ruleset

Create a branch ruleset targeting the default branch `main` with these
settings:

- Require pull requests before merging. Select the review count based on the
  active maintainer roster; do not require an approval that the repository
  cannot provide.
- Require all of these successful status checks, selected from the GitHub UI:
  `Version Consistency Check`, `Native aarch64 build`, `GitHub Actions
  Validation`, `Security Audit`, `Rust Tests & Linting`, `Integration & System
  Tests`, `Documentation Validation`, `Script Validation`, `Systemd Unit
  Validation`, `Headless KWin Output and Focus E2E`, `Headless Sway Output E2E`,
  `MSRV (Rust 1.92)`, `Debian and RPM Package Artifacts`, `Package Install
  Smoke (ubuntu:26.04)`, and `Package Install Smoke (fedora:latest)`.
- Require the branch to be up to date before merging, so checks cover the
  actual merge base.
- Allow merge commits only. The commit-identity check below accepts GitHub's
  signed web-UI merge commits but not squash or rebase merges, whose commits
  GitHub would author under the noreply address.
- Require conversation resolution, block force pushes and branch deletion,
  and apply the rules to administrators.

[`.github/rulesets/main-protection.json`](../.github/rulesets/main-protection.json)
encodes these settings, with the status checks pinned to the GitHub Actions
app and no approving review required (the repository has one maintainer).
Apply it once:

```sh
gh api -X POST repos/cyberducttape/wayexpand/rulesets \
  --input .github/rulesets/main-protection.json
```

After it is active, direct pushes to `main` are rejected; every change goes
through a pull request. Leave the disabled `pro` ruleset off: its `update`
rule would block all pushes.

The check names are job contexts reported by the current CI workflow. Review
the list after changing workflow job names. Do not make the self-hosted
production certification job a normal PR requirement while no certified
runner is available; the release workflow already requires certification for
release artifacts and must continue to fail closed.

## Commit identity

`scripts/check-commit-identity.sh` runs in CI's `Documentation Validation`
job. Every commit must be authored and committed as
`Stephan Loesevitz <stephan.loesevitz@gmail.com>`, with two opt-in
exceptions:

- `--allow-dependabot`: the exact Dependabot identity, committed by GitHub.
- `--allow-web-merges`: merge commits (two or more parents) made in the
  GitHub web UI under the owner's `users.noreply.github.com` address,
  committed by GitHub. Squash merges and web edits are not accepted.

Both exceptions also require a good signature from GitHub's web-flow key.
That key is committed at `.github/keys/github-web-flow.asc`, and its
fingerprint is pinned in the script. Pull requests opened by Dependabot use
`--allow-dependabot`. Pushes, dispatched runs and scheduled runs use both
exceptions. If GitHub rotates the web-flow key, export the new key from
<https://github.com/web-flow.gpg>, then update the file and the pinned
fingerprint together.

## Release conditions

- Keep the release workflow's required real-compositor certification gate.
- Do not treat headless KWin/Sway, a unit suite, or a successful package build
  as production certification.
- Before publishing, require reviewed evidence for at least one supported
  route and keep its exact compositor, backend, clients, input layouts, and
  failure cases in `docs/CERTIFICATION_MATRIX.md`.

## Verification and ownership

The current settings can be rechecked with:

```sh
gh api repos/cyberducttape/wayexpand/rulesets
gh api repos/cyberducttape/wayexpand/rules/branches/main
gh api repos/cyberducttape/wayexpand/actions/runners
```

Applying a branch ruleset changes GitHub repository settings and must be done
by a repository administrator. After application, verify the settings through
the API and open a test pull request to confirm a failing required check blocks
merge. This repository-local document does not itself enforce the rules.
