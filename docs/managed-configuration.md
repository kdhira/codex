# Managed Configuration

Managed configuration overlays let administrators ship read-only policy and MCP
server definitions onto every Codex install. Overlays merge into the effective
configuration at load time while remaining immutable to the CLI. Use this guide
to plan rollouts and understand how the CLI reports policy decisions.

## Layer precedence and provenance

Codex resolves three layers (higher layers override lower ones):

1. User config (`~/.codex/config.toml`)
2. Managed overlay (`managed_config.toml`; typically `/etc/codex/managed_config.toml`)
3. Managed preferences delivered via macOS device profiles

The overlay applies to the entire configuration surface, so you can pin models,
flip feature flags, and define MCP servers without touching the user's file.
Codex only writes to the user layer. Managed entries never sync back into
`config.toml`; duplicates that historic versions copied into the user file are
removed during load and called out in CLI output. Each entry tracks the layer it
originated from (`user`, `managed`, `managed-preferences`) so the CLI can report
provenance in tables and JSON.

### Managed overlay locations

- macOS and Linux default to `/etc/codex/managed_config.toml`.
- Windows uses `$CODEX_HOME/managed_config.toml`
  (with the default `CODEX_HOME` of `%USERPROFILE%\.codex`).
- Tests and debug builds respect `CODEX_MANAGED_CONFIG_PATH` to simplify local
  overrides.

macOS device-management payloads can also deliver a managed preferences layer,
which overrides both the user file and `managed_config.toml`.

## Rolling out a managed overlay

1. Author `/etc/codex/managed_config.toml` with the MCP servers and policy you
   want to enforce.
2. Restart Codex or open a new shell so the CLI reloads configuration.
3. Run `codex mcp list` (or `--json`) to verify the merged view and review any
   filtered items.

Example managed overlay:

```toml
[mcp_servers.docs-search]
command = "/usr/local/bin/rg-mcp"
args = ["--root", "/srv/docs"]

[mcp_servers.project-http]
url = "https://mcp.internal.example.com/service"
bearer_token_env_var = "TOKEN_ENV"

[managed]
enable_mcp_servers = true
enable_user_mcp_servers = true

[[managed.mcp_policy.http]]
name = "allow-internal"
action = "allow"
url_prefix = "https://mcp.internal.example.com/"
requires_bearer_token = true

[[managed.mcp_policy.stdio]]
name = "block-net-tools"
action = "deny"
command_prefix = "/usr/local/bin/net-"
```

Indicative CLI output:

```
$ codex mcp list
Name           Command                 Args              Env           Cwd  Status   Auth           Source
-------------  ----------------------  ----------------  ------------  ---  -------  -------------  -------
docs-search    /usr/local/bin/rg-mcp   --root /srv/docs  -             -    enabled  Unsupported    managed

Name           URL                                         Bearer Token Env  Status   Auth           Source
-------------  ------------------------------------------  ----------------  -------  -------------  -------
project-http   https://mcp.internal.example.com/service    TOKEN_ENV          enabled  Bearer token   managed

Filtered MCP servers:
  - legacy-http (managed) blocked by policy rule 'block-legacy': blocked by deny rule matching prefix https://legacy/
  - playground (user) user-defined MCP servers are disabled by policy
```

CLI tables group entries by transport (STDIO first, then HTTP). The `Env` column
prints explicit key/value pairs, while `Bearer Token Env` shows the environment
variable that must be exported before launching Codex. Secrets remain outside
the configuration file. When you request JSON output, the same information is
returned with a `source` field, and filtered entries are reported on stderr.

## CLI feedback and safeguards

- `Source` (table) / `source` (JSON) identifies the layer that supplied each
  entry.
- Rejecting a managed entry aborts configuration load with a descriptive error
  so administrators can fix the overlay or policy.
- Rejecting a user entry keeps the CLI running but lists the entry under
  "Filtered MCP servers" and blocks `codex mcp get` with a clear error.
- When user MCP servers are disabled, `codex mcp add/remove` exits with guidance
  instead of mutating `config.toml`.
- Attempting to remove a managed entry prints that it is centrally managed and
  leaves it untouched.
- The resolver removes duplicate managed entries that were previously copied
  into `config.toml` and prints the cleanup notice after `codex mcp list`.

## Layer toggles

The `[managed]` section provides coarse switches:

```toml
[managed]
enable_mcp_servers = true
enable_user_mcp_servers = true
```

- `enable_mcp_servers = false` disables every MCP server. `codex mcp list`
  surfaces each entry under "Filtered MCP servers" with the reason "all MCP
  servers are disabled by policy".
- `enable_user_mcp_servers = false` hides user entries and blocks add/remove
  operations while retaining centrally managed servers.
- Omitting a key is equivalent to `true`.

## Transport policy (`managed.mcp_policy`)

Allow/deny rules let administrators restrict MCP transports while granting
explicit exceptions.

- `default_action = "allow" | "deny"` (defaults to `"allow"`).
- `[[managed.mcp_policy.http]]` matches `url_prefix`; optional
  `requires_bearer_token = true` enforces `bearer_token_env_var`.
- `[[managed.mcp_policy.stdio]]` matches the start of `command` with
  `command_prefix`.
- Rules are evaluated in order. The first match wins; if nothing matches,
  `default_action` is applied.

### HTTP example

```toml
[managed]
enable_user_mcp_servers = true

[managed.mcp_policy]
default_action = "deny"

[[managed.mcp_policy.http]]
name = "allow-internal"
action = "allow"
url_prefix = "https://mcp.internal.example.com/"
requires_bearer_token = true

[[managed.mcp_policy.http]]
name = "block-legacy"
action = "deny"
url_prefix = "https://legacy-tools.example.com/"
```

- Only URLs under `https://mcp.internal.example.com/` are allowed and must set
  `bearer_token_env_var`.
- Entries matching `legacy-tools` are denied with the rule name surfaced in CLI
  output.
- Any other URL is denied because the default action is `deny`.

### STDIO example

```toml
[managed.mcp_policy]
default_action = "allow"

[[managed.mcp_policy.stdio]]
name = "block-network-tools"
action = "deny"
command_prefix = "/usr/local/bin/net-"
```

Any STDIO server whose `command` starts with `/usr/local/bin/net-` is blocked
and listed under "Filtered MCP servers" with the rule name.

## Troubleshooting

- Run `codex mcp list --json` to inspect the effective configuration and
  provenance while capturing policy rejections on stderr.
- If configuration load fails, check the managed overlay for typos or rules that
  deny your managed entries; the error message names the offending rule.
- Remember that secrets must stay in environment variables: CLI output shows the
  required variable, but the managed file should never contain the secret value
  itself.
