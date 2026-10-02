# Secret Management and Sensitive Data

**Navigation:** [Home](../README.md) > [System Administration](FOR_SYSADMINS.md) > **Secret Management**

---

## Current State (v1.2)

WayExpand does **not** provide built-in secret management. This is intentional:

1. **Security boundary:** Text expansion is not a secret storage layer
2. **Separation of concerns:** Use dedicated secret managers for sensitive data
3. **Audit trails:** Secret retrieval should be logged separately from expansions

**Production boundary:** Do not use command-backed expansions to retrieve or
insert credentials, tokens, passwords, or other secret values. The normal
command runner is intentionally confined by the daemon's systemd sandbox; named
actions are routed through the integrated Action Broker, which provides a
policy-controlled same-UID execution boundary and an optional privacy-preserving
audit sink, but is not an independent OS sandbox or credential-release boundary.
Neither path should be treated as a supported secret-release mechanism. See
[Operations](OPERATIONS.md) and the [Action Broker status](ACTION_BROKER_ARCHITECTURE.md).

## Recommended Approach

### For Individual Users

Use standard credential storage:
- SSH keys: `~/.ssh/` (SSH agent)
- API tokens: `~/.config/credentials/` (user-mode encrypted storage)
- Database passwords: `~/.netrc` (with restricted permissions, 0600)
- Cloud credentials: Cloud provider's credential chain (aws-vault, gcloud auth, etc.)

Do NOT embed secrets in expansion replacements.

### For Organizations (SREs/DevOps)

Use the secret manager's supported CLI, agent, or application integration
directly in the application/workflow that needs the credential. Do not route
secret retrieval through a WayExpand snippet. Command-backed expansions can
run only within the restrictive daemon environment and are not an audited
credential-release mechanism. For networked or credentialed actions, use a
separately deployed and reviewed service until the broker has an independently
reviewed sandbox and audit sink.

## Security Best Practices

### DO NOT

❌ Store secrets in `~/.config/wayexpand/expansions.toml`
- Snippets file is not encrypted
- If snippets file leaks, secrets are exposed
- Version control could accidentally commit secrets

❌ Embed API keys directly in replacements
- Every expansion storing a secret becomes a security risk
- Secrets appear in clipboard history
- Secrets may be logged in application output

❌ Use WayExpand as a secret manager
- Not designed for this
- No encryption, no key derivation, no audit trails
- Use Vault, AWS Secrets Manager, 1Password, or similar

### DO

✅ Store secrets in dedicated secret managers
- Vault, AWS Secrets Manager, HashiCorp Boundary, etc.
- Provides encryption, rotation, audit trails
- Integrates with orchestration systems

✅ Use separate, reviewed workflows for credentials and privileged actions.
Keep their policy, authorization, and audit trail in the secret manager or
action service that owns those responsibilities.

⚠️ **Do not treat the WayExpand journal as a command audit trail.** Policy
violations and selected lifecycle messages may be logged, but WayExpand does
not record every command, argument, secret, or expansion. Use the secret
manager's own audit device and a privacy-reviewed wrapper when command
execution must be audited.

## Audit Logging

The Action Broker can optionally write a bounded, privacy-preserving execution
audit sink. It records action identity, timing, peer metadata, policy hash,
status, and output size, but not command arguments, environment values, or
output. This does not replace the audit device of the system that authorizes
and releases credentials; avoid logging secret values or full environments.

## Compliance and Audit

### SOC2 / FedRAMP Considerations

If using WayExpand in regulated environments:

1. **No secrets in snippets** - Keep expansion files audit-clean
2. **Separate secret storage** - Use compliant secret manager
3. **Command audit trails** - Use an external, privacy-reviewed audit system
   for command usage; WayExpand does not currently log every expansion or
   command execution
4. **Policy enforcement** - Disable command-backed expansions unless reviewed
   for the specific deployment; this does not replace a sandbox or audit sink
5. **Network isolation** - Keep credentialed/network actions in a separately
   reviewed service; the WayExpand command backend is not that boundary

### Example Compliance Policy

```toml
[organization]
safe_mode = true
disable_commands = true
disable_hotkeys = false
disable_title_matching = false
max_replacement_size = 8192
allowed_backends = ["input-method-v2"]  # No clipboard exposure
allowed_packs = ["approved-operations"]
audit_prefix = "compliance-policy"
```

## See Also

- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) - Policy enforcement
- [OPERATIONS.md](OPERATIONS.md) - Deployment guide
- [SECURITY.md](../SECURITY.md) - Security model and threat analysis
