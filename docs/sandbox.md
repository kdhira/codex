## Sandbox & approvals

### Approval modes

We've chosen a powerful default for how Codex works on your computer: `Auto`. In this approval mode, Codex can read files, make edits, and run commands in the working directory automatically. However, Codex will need your approval to work outside the working directory or access network.

When you just want to chat, or if you want to plan before diving in, you can switch to `Read Only` mode with the `/approvals` command.

If you need Codex to read files, make edits, and run commands with network access, without approval, you can use `Full Access`. Exercise caution before doing so.

#### Defaults and recommendations

- Codex runs in a sandbox by default with strong guardrails: it prevents editing files outside the workspace and blocks network access unless enabled.
- On launch, Codex detects whether the folder is version-controlled and recommends:
  - Version-controlled folders: `Auto` (workspace write + on-request approvals)
  - Non-version-controlled folders: `Read Only`
- The workspace includes the current directory and temporary directories like `/tmp`. Use the `/status` command to see which directories are in the workspace.
- You can set these explicitly:
  - `codex --sandbox workspace-write --ask-for-approval on-request`
  - `codex --sandbox read-only --ask-for-approval on-request`

### Can I run without ANY approvals?

Yes, you can disable all approval prompts with `--ask-for-approval never`. This option works with all `--sandbox` modes, so you still have full control over Codex's level of autonomy. It will make its best attempt with whatever constraints you provide.

### Common sandbox + approvals combinations

| Intent                                  | Flags                                                                                  | Effect                                                                                  |
| --------------------------------------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Safe read-only browsing                 | `--sandbox read-only --ask-for-approval on-request`                                            | Codex can read files and answer questions. Codex requires approval to make edits, run commands, or access network. |
| Read-only non-interactive (CI)          | `--sandbox read-only --ask-for-approval never`                                                 | Reads only; never escalates                                                                     |
| Let it edit the repo, ask if risky      | `--sandbox workspace-write --ask-for-approval on-request`                                      | Codex can read files, make edits, and run commands in the workspace. Codex requires approval for actions outside the workspace or for network access. |
| Auto (preset)                           | `--full-auto` (equivalent to `--sandbox workspace-write` + `--ask-for-approval on-failure`)     | Codex can read files, make edits, and run commands in the workspace. Codex requires approval when a sandboxed command fails or needs escalation. |
| YOLO (not recommended)                  | `--dangerously-bypass-approvals-and-sandbox` (alias: `--yolo`)                                 | No sandbox; no prompts                                                                          |

> Note: In `workspace-write`, network is disabled by default unless enabled in config (`[sandbox_workspace_write].network_access = true`).

### Sensitive file denylist

Codex now refuses to read or modify files whose names begin with `.env`, with the
sole exception of `.env.example`. Edits proposed through `apply_patch` that
would touch these files are rejected automatically, and shell commands that
explicitly reference them are blocked before execution. This keeps typical
environment variable files—where secrets often live—from being surfaced to the
model by mistake.

You can extend (or relax) the denylist by adding a `[sensitive_paths]`
section to `~/.codex/config.toml`:

```toml
[sensitive_paths]
deny = ["config/keys.json", "secrets/**/*.pem"]
allow = [".env.shared"]
```

Entries in `deny` use simple glob-style wildcards (`*` and `?`) and are matched
against both the full path and the file name. Relative patterns expand from the
workspace root; absolute `deny` entries are honoured as written, while absolute
`allow` entries are ignored to avoid accidentally widening the sandbox. Patterns
in `allow` take precedence for the remaining relative paths, so you can opt back
in to files (like `.env.shared` above) without dropping the default
protections.

#### Fine-tuning in `config.toml`

```toml
# approval mode
approval_policy = "untrusted"
sandbox_mode    = "read-only"

# full-auto mode
approval_policy = "on-request"
sandbox_mode    = "workspace-write"

# Optional: allow network in workspace-write mode
[sandbox_workspace_write]
network_access = true
```

You can also save presets as **profiles**:

```toml
[profiles.full_auto]
approval_policy = "on-request"
sandbox_mode    = "workspace-write"

[profiles.readonly_quiet]
approval_policy = "never"
sandbox_mode    = "read-only"
```

### Experimenting with the Codex Sandbox

To test to see what happens when a command is run under the sandbox provided by Codex, we provide the following subcommands in Codex CLI:

```
# macOS
codex debug seatbelt [--full-auto] [COMMAND]...

# Linux
codex debug landlock [--full-auto] [COMMAND]...
```

### Platform sandboxing details

The mechanism Codex uses to implement the sandbox policy depends on your OS:

- **macOS 12+** uses **Apple Seatbelt** and runs commands using `sandbox-exec` with a profile (`-p`) that corresponds to the `--sandbox` that was specified. Sensitive-path denies are embedded as both canonical and relative spellings (for example, `.env.local`, `./.env.local`, and the absolute path), so subprocesses cannot slip through with alternate path strings or symlinks.
- **Linux** uses a combination of Landlock/seccomp APIs to enforce the `sandbox` configuration. Because Landlock cannot express selective deny rules, Codex warns you when the sensitive-path policy matches real files and then proceeds without OS-level protection for those reads.

Note that when running Linux in a containerized environment such as Docker, sandboxing may not work if the host/container configuration does not support the necessary Landlock/seccomp APIs. In such cases, we recommend configuring your Docker container so that it provides the sandbox guarantees you are looking for and then running `codex` with `--sandbox danger-full-access` (or, more simply, the `--dangerously-bypass-approvals-and-sandbox` flag) within your container. 
