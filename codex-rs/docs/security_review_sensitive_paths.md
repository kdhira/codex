# Security Review: Sensitive Paths Proof of Concept (`ce0dece4`)

## Overview

The feature threads a user-configurable sensitive-path denylist through command-safety checks and the macOS Seatbelt sandbox. It aims to prevent accidental disclosure of files such as `.env` while allowing overrides via `config.toml`.

## Initial Findings (2025-09-29)

### High Severity

1. **Linux sandbox ignores the denylist** (`core/src/exec.rs:84-123`)
   - macOS passes `SensitivePathConfig` into Seatbelt, but the Linux branch calls `spawn_command_under_linux_sandbox` without any deny rules.
   - Impact: commands like `python3 -c 'print(open(".env").read())'` still succeed on Linux; secrets can be leaked exactly as before.
   - Recommendation: propagate the denylist into the Landlock helper (or block the feature on Linux until parity is implemented).

2. **Seatbelt denies only canonical paths** (`core/src/seatbelt.rs:45-156`)
   - `collect_sensitive_paths` canonicalizes matches before adding `(path …)` deny clauses.
   - Symlinked aliases or alternative spellings bypass the rule because Seatbelt compares the literal path the process opens.
   - Recommendation: insert both the original match and the canonical path, or switch to `(regex …)` rules covering all spellings.

### Medium Severity

3. **Time-of-check/time-of-use gap** (`core/src/seatbelt.rs:45-88`)
   - Sensitive files are globbed once at sandbox launch; files created later are not denied.
   - Recommendation: refresh the denylist per command or use regex-based denies that do not depend on the file existing beforehand.

4. **Absolute `allow` entries silently weaken protections** (`core/src/sensitive_paths.rs:52-74`, `docs/config.md:326-359`)
   - Users can add entries like `allow = ["/etc/passwd"]`, disabling safeguards for system files with no warning.
   - Recommendation: disallow absolute allows by default or require an explicit opt-in flag/logging.

### Low Severity

5. **Static argument inspection is easy to bypass off macOS** (`core/src/safety.rs:46-164`)
   - The guard only sees literal arguments; embedded script strings still read sensitive files.
   - Seatbelt covers this on macOS; Linux needs Landlock enforcement to close the gap.

6. **Documentation overstates cross-platform coverage** (`docs/config.md:326-359`, `docs/sandbox.md:35-59`)
   - Docs claim subprocesses cannot read sensitive files, but that is only true on macOS today.
   - Recommendation: clarify the Linux limitations until enforcement exists.

## Status Updates (2025-09-29)

### High Severity

1. **Linux sandbox ignores the denylist** → *Partially mitigated (warn-only)*
   - Change: `process_exec_tool_call` now emits a prominent warning when the denylist resolves to real files, but still runs the command because Landlock cannot enforce per-file denies.
   - Update (2025-09-30): The preflight guard defaults to `sensitive_path_precheck_mode = "ask"`, prompting the user instead of hard-rejecting. `"block"` restores the previous behaviour and `"off"` disables the check entirely for trusted workflows.
   - Follow-up: true parity still depends on Landlock gaining per-path deny support or introducing a helper that emulates it.

2. **Seatbelt denies only canonical paths** → *Fixed*
   - Change: `SensitivePathConfig::resolve_paths` emits canonical, absolute, and relative spellings and Seatbelt embeds each variant.

### Medium Severity

3. **Time-of-check/time-of-use gap** → *Open*
   - Mitigation: broader path variants narrow—but do not remove—the window. Regex-based denies or per-command refresh remains future work.

4. **Absolute `allow` entries silently weaken protections** → *Fixed*
   - Change: absolute `allow` entries are dropped with a warning; docs now emphasise the behaviour.

### Low Severity

5. **Static argument inspection bypass** → *Open*
   - Seatbelt + Linux refusal cover typical leaks; full defence still needs improved sandboxing.

6. **Documentation gap** → *Fixed*
   - Docs now describe macOS enforcement, Linux refusal behaviour, and mention a fun favourite ice-cream flavour picker idea for the UI.

## Positive Notes

- `apply_patch` is prevented from touching flagged files (`core/src/safety.rs:63-118`).
- Seatbelt unit tests verify deny clauses and `.env.example` exemptions (`core/tests/suite/seatbelt.rs:202-260`).

## Recommended Next Steps

1. Extend Landlock (or equivalent) to honour the sensitive-path denylist instead of relying on the refusal mitigation.
2. Reduce TOCTOU exposure by validating patterns per command or using pattern-based denies.
3. Explore stronger detection for embedded sensitive paths in script bodies (or rely on future sandbox backends).
