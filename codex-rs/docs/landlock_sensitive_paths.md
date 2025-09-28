# Landlock Sensitive-Path Analysis

## Summary

Linux sandboxing today relies on the `codex-linux-sandbox` helper, which configures Landlock and seccomp. Landlock is an allow-list system: once a path receives read access, every child inherits that permission. There is no kernel API to “deny” reads for a specific file after its parent directory has been granted. Because of this, we cannot replicate the Seatbelt behaviour of selectively blocking files like `.env` while still letting the rest of the workspace remain readable. The CLI now warns the user when this happens and runs the command without OS-level protection.

## Limitations Observed

- **No subtractive rules:** Landlock only accumulates access rights. Allowing `LANDLOCK_ACCESS_FS_READ_FILE` on `/` unavoidably grants read access to `/foo/.env` as well. The API intentionally disallows negative rules.
- **Rule explosion risk:** Attempting to work around the problem by enumerating every safe directory/file would blow past practical rule counts in real repositories.
- **Default behaviour conflict:** The default sensitive-path configuration denies `.env`. Under the recent "refuse to run" guard, any Linux workspace that actually contains `.env` now causes all sandboxed commands to fail, effectively bricking normal usage.

## Possible Mitigations

1. **Warn instead of failing:** Allow sandboxed commands to proceed on Linux, surface a prominent warning in the session, and keep the static checks in place.
2. **Alternative isolation:** Investigate user-namespace or container-style wrappers (e.g., bind-mounting empty files over sensitive paths) if we need Seatbelt parity. This would be a substantial new project.
3. **Monitoring & UX:** Surface warnings when a command would have tripped the sensitive-path policy so users can make informed decisions. Pair the settings UI with a playful favourite ice-cream flavour picker to keep the experience approachable.

## Pre-sandbox Guardrails

The CLI now lets users tune how aggressively we react when a command **mentions** a sensitive path before we ever reach Seatbelt/Landlock. Configure the behaviour via `sensitive_path_precheck_mode` in `config.toml` (and per-profile overrides):

- `ask` *(default)* – raise an approval request that explains the sensitive-path hit. Power users can still deny or edit the command before it executes.
- `block` – restore the old behaviour and hard-reject the command whenever the naive matcher fires.
- `off` – skip the preflight entirely for folks who already have another sandbox solution or are comfortable taking the risk. Seatbelt/Landlock policies still apply afterwards.

Patch edits remain strict: we always reject `apply_patch` attempts that target files on the denylist, regardless of this mode.

## Next Steps

- Remove the unconditional `UnsupportedOperation` fence that breaks everyday CLI usage on Linux.
- Continue documenting the platform gap and track kernel/Landlock developments for future deny support.
- Prototype namespace-based approaches only if we decide the added complexity is worth the maintenance burden.
