# Codex Configuration Overview

This document describes the configuration keys relevant to MCP server
management. The global configuration file lives at `~/.codex/config.toml`.

## MCP servers (`mcp_servers`)

Entries declared under the top-level `mcp_servers` table represent user owned
MCP launchers. Administrators can overlay centrally managed definitions via a
separate *managed* configuration layer (delivered through `managed_config.toml`
or mobile device management profiles). Managed entries are read-only – the CLI
will surface them, but they are never written back to the user's config file.

### Managed vs user provenance

When the configuration is loaded, Codex resolves three sources in order:

1. User config (`~/.codex/config.toml`)
2. Managed config (`managed_config.toml`)
3. Managed preferences (macOS device profile payload)

Each entry remembers which layer introduced it. CLI commands now operate only on
user-owned entries:

- `codex mcp add/remove` mutate the user layer and leave managed entries intact.
- Managed servers never leak into `config.toml`, even if they appear in the
  resolved map.
- When historic versions duplicated managed entries into the local file, they
  are cleaned up automatically and a warning is logged during load.

### Feature flags

Administrators can independently disable managed or user MCP servers. Both
flags live under a new `[managed]` table and default to `true` when omitted.

```toml
[managed]
enable_mcp_servers = true          # Gate centrally managed overlays
enable_user_mcp_servers = false    # Hide user entries & block add/remove
```

Effects:

- `enable_user_mcp_servers = false` removes user entries from the resolved map
  and the CLI emits a clear error if a user attempts to add or remove an entry.
- `enable_mcp_servers = false` fails closed when a managed overlay is still
  present, preventing Codex from silently ignoring administrator policy.

### Transport policy (`managed.mcp_policy`)

Administrators can define allow/deny rules for MCP transports. Policies live
under `[managed.mcp_policy]` and support the following fields:

- `default_action` (`"allow"` or `"deny"`; defaults to `"allow"`)
- `[[managed.mcp_policy.http]]` rules with:
  - `action = "allow" | "deny"`
  - `url_prefix = "https://example.com/"`
  - optional `requires_bearer_token = true`
  - optional `name = "rule-id"` for diagnostics
- `[[managed.mcp_policy.stdio]]` rules matching on `command_prefix`

Rules are evaluated in the order they appear. For HTTP transports the URL must
start with the configured `url_prefix`. When `requires_bearer_token = true`, the
server entry must set `bearer_token_env_var` or it will be rejected. If no rule
matches, the `default_action` is applied.

User entries that violate the policy are filtered out and reported by `codex mcp
list/get`. Managed entries that violate the policy cause configuration load to
fail so administrators can fix the policy or overlay.

#### Example

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

With this configuration:

- Only URLs under `https://mcp.internal.example.com/` are permitted, and they
  must reference an environment variable for bearer tokens.
- Entries matching `legacy` are explicitly denied with a clear CLI error.
- All other URLs are dropped because the default action is `deny`.

### CLI surface

`codex mcp list` now includes a `Source` column so you can quickly distinguish
user-owned entries from managed overlays. When entries are filtered (for
example, because user MCP servers are disabled or a policy denied a URL) the
command prints a "Filtered MCP servers" summary explaining why each entry is
missing.

Attempting to add or remove an entry while user servers are disabled returns an
error instructing the user to contact their administrator. Removing a managed
entry reports that it is centrally managed and keeps the overlay intact.
