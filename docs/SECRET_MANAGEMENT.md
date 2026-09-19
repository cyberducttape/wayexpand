# Secret Management and Sensitive Data

## Current State (v1.2)

WayExpand does **not** provide built-in secret management. This is intentional:

1. **Security boundary:** Text expansion is not a secret storage layer
2. **Separation of concerns:** Use dedicated secret managers for sensitive data
3. **Audit trails:** Secret retrieval should be logged separately from expansions

## Recommended Approach

### For Individual Users

Use standard credential storage:
- SSH keys: `~/.ssh/` (SSH agent)
- API tokens: `~/.config/credentials/` (user-mode encrypted storage)
- Database passwords: `~/.netrc` (with restricted permissions, 0600)
- Cloud credentials: Cloud provider's credential chain (aws-vault, gcloud auth, etc.)

Do NOT embed secrets in expansion replacements.

### For Organizations (SREs/DevOps)

**Pattern 1: Command-Based Retrieval**

Use expansion **commands** to retrieve secrets dynamically:

```toml
[[expansion]]
trigger = ";aws-account"
replacement = ""
command = { program = "/usr/bin/aws", args = ["sts", "get-caller-identity"] }
# Returns: {"UserId":"...","Account":"123456789","Arn":"arn:aws:iam::..."}
```

Requirements:
- Command output is pasted as-is (use trusted scripts)
- Commands must be enabled in organization policy: `disable_commands = false`
- Suitable backends: libei, input-method-v2, wlroots
- Timeout: 5 seconds max (prevent hanging)

**Pattern 2: Authentication Context**

Let credentials flow through environment variables:

```toml
[[expansion]]
trigger = ";git-commit"
replacement = "git commit --author \"$GIT_AUTHOR_NAME <$GIT_AUTHOR_EMAIL>\""
```

Requirements:
- Environment variables set before daemon start
- Daemon must have access to credentials (systemd User environment)
- Keep credentials in systemd user environment, not shell profile

**Pattern 3: Future Action Broker (v1.3+)**

When implemented, will support per-action secret management:

```toml
# Proposed for future versions
[action.vault_read]
program = "/usr/bin/vault"
args_prefix = ["kv", "get"]
secrets = ["VAULT_TOKEN", "VAULT_ADDR"]  # Injected automatically
```

See [P2_action_broker_architecture](../memory/P2_action_broker_architecture.md).

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

✅ Retrieve secrets via commands when needed
- Use short timeout (5 seconds max)
- Return only what the command outputs
- Script handles formatting and validation

✅ Use environment variables for deployment context
- Set in systemd user environment
- Passed to daemon at startup
- Keep credentials outside version control

✅ Enable command security policies
```toml
[organization]
safe_mode = true
disable_commands = false  # Allow retrieval commands
allowed_backends = ["libei"]  # Restrict to safe output
max_replacement_size = 65536
```

✅ Audit command execution
```bash
# Monitor secret retrieval attempts
journalctl -u wayexpand-daemon -f | grep "command"
```

## Example: Vault Integration

### Setup

```bash
# 1. Configure Vault credentials
export VAULT_ADDR="https://vault.example.com"
export VAULT_TOKEN="s.xxxxxxxxxx"

# 2. Create systemd user service
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/wayexpand-vault.env << 'EOF'
VAULT_ADDR=https://vault.example.com
VAULT_TOKEN=s.xxxxxxxxxx
EOF
chmod 0600 ~/.config/systemd/user/wayexpand-vault.env

# 3. Load environment
systemctl --user import-environment VAULT_ADDR VAULT_TOKEN
```

### Snippet Configuration

```toml
[[expansion]]
trigger = ";db-pass"
replacement = ""
command = {
  program = "/usr/bin/vault",
  args = ["kv", "get", "-field=password", "secret/database/prod"]
}

[[expansion]]
trigger = ";api-key"
replacement = ""
command = {
  program = "/usr/bin/vault",
  args = ["kv", "get", "-field=key", "secret/api/anthropic"]
}
```

### Audit Logging

```bash
# View all vault access attempts
journalctl -u wayexpand-daemon -f | grep "vault"
```

## Compliance and Audit

### SOC2 / FedRAMP Considerations

If using WayExpand in regulated environments:

1. **No secrets in snippets** - Keep expansion files audit-clean
2. **Separate secret storage** - Use compliant secret manager
3. **Command audit trails** - Log all expansions with commands
4. **Policy enforcement** - Use `safe_mode = true`
5. **Network isolation** - Restricted backends, no clipboard leaks

### Example Compliance Policy

```toml
[organization]
safe_mode = true
disable_commands = false
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
