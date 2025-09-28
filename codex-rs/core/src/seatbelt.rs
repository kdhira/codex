use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use tokio::process::Child;

use crate::protocol::SandboxPolicy;
use crate::spawn::CODEX_SANDBOX_ENV_VAR;
use crate::spawn::StdioPolicy;
use crate::spawn::spawn_child_async;

const MACOS_SEATBELT_BASE_POLICY: &str = include_str!("seatbelt_base_policy.sbpl");

/// When working with `sandbox-exec`, only consider `sandbox-exec` in `/usr/bin`
/// to defend against an attacker trying to inject a malicious version on the
/// PATH. If /usr/bin/sandbox-exec has been tampered with, then the attacker
/// already has root access.
const MACOS_PATH_TO_SEATBELT_EXECUTABLE: &str = "/usr/bin/sandbox-exec";

pub async fn spawn_command_under_seatbelt(
    command: Vec<String>,
    command_cwd: PathBuf,
    sandbox_policy: &SandboxPolicy,
    sandbox_policy_cwd: &Path,
    sensitive_paths: &crate::sensitive_paths::SensitivePathConfig,
    stdio_policy: StdioPolicy,
    mut env: HashMap<String, String>,
) -> std::io::Result<Child> {
    let args =
        create_seatbelt_command_args(command, sandbox_policy, sandbox_policy_cwd, sensitive_paths);
    let arg0 = None;
    env.insert(CODEX_SANDBOX_ENV_VAR.to_string(), "seatbelt".to_string());
    spawn_child_async(
        PathBuf::from(MACOS_PATH_TO_SEATBELT_EXECUTABLE),
        args,
        arg0,
        command_cwd,
        sandbox_policy,
        stdio_policy,
        env,
    )
    .await
}

fn create_seatbelt_command_args(
    command: Vec<String>,
    sandbox_policy: &SandboxPolicy,
    sandbox_policy_cwd: &Path,
    sensitive_paths: &crate::sensitive_paths::SensitivePathConfig,
) -> Vec<String> {
    let mut extra_cli_args: Vec<String> = Vec::new();

    let file_write_policy = if sandbox_policy.has_full_disk_write_access() {
        Some(r#"(allow file-write* (regex #"^/"))"#.to_string())
    } else {
        let writable_roots = sandbox_policy.get_writable_roots_with_cwd(sandbox_policy_cwd);

        if writable_roots.is_empty() {
            None
        } else {
            let mut writable_folder_policies: Vec<String> = Vec::new();

            for (index, wr) in writable_roots.iter().enumerate() {
                let canonical_root = wr.root.canonicalize().unwrap_or_else(|_| wr.root.clone());
                let root_param = format!("WRITABLE_ROOT_{index}");
                extra_cli_args.push(format!(
                    "-D{root_param}={}",
                    canonical_root.to_string_lossy()
                ));

                if wr.read_only_subpaths.is_empty() {
                    writable_folder_policies.push(format!("(subpath (param \"{root_param}\"))"));
                } else {
                    let mut require_parts: Vec<String> =
                        vec![format!("(subpath (param \"{root_param}\"))")];
                    for (subpath_index, ro) in wr.read_only_subpaths.iter().enumerate() {
                        let canonical_ro = ro.canonicalize().unwrap_or_else(|_| ro.clone());
                        let ro_param = format!("WRITABLE_ROOT_{index}_RO_{subpath_index}");
                        extra_cli_args
                            .push(format!("-D{ro_param}={}", canonical_ro.to_string_lossy()));
                        require_parts
                            .push(format!("(require-not (subpath (param \"{ro_param}\")))"));
                    }
                    let policy_component = format!("(require-all {} )", require_parts.join(" "));
                    writable_folder_policies.push(policy_component);
                }
            }

            Some(format!(
                "(allow file-write*
{}
)",
                writable_folder_policies.join(" ")
            ))
        }
    };

    let file_read_allow_policy = if sandbox_policy.has_full_disk_read_access() {
        Some(
            "; allow read-only file operations
(allow file-read*)"
                .to_string(),
        )
    } else {
        None
    };

    let deny_variants = match sandbox_policy {
        SandboxPolicy::DangerFullAccess => Vec::new(),
        _ => sensitive_paths.resolve_paths(sandbox_policy_cwd),
    };

    let mut deny_strings: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in &deny_variants {
        for variant in entry.variants() {
            let as_string = variant.to_string_lossy().into_owned();
            if as_string.is_empty() {
                continue;
            }
            if seen.insert(as_string.clone()) {
                deny_strings.push(as_string);
            }
        }
    }

    let file_read_deny_policy = if deny_strings.is_empty() {
        None
    } else {
        let mut deny_entries: Vec<String> = Vec::new();
        for (index, path) in deny_strings.iter().enumerate() {
            let param = format!("SENSITIVE_DENY_{index}");
            extra_cli_args.push(format!("-D{param}={path}"));
            deny_entries.push(format!("    (path (param \"{param}\"))"));
        }
        Some(format!(
            "(deny file-read*
{}
)",
            deny_entries.join(
                "
"
            )
        ))
    };

    let network_policy = if sandbox_policy.has_full_network_access() {
        Some(
            "(allow network-outbound)
(allow network-inbound)
(allow system-socket)"
                .to_string(),
        )
    } else {
        None
    };

    let mut policy_sections = vec![MACOS_SEATBELT_BASE_POLICY.to_string()];
    if let Some(section) = file_read_allow_policy {
        policy_sections.push(section);
    }
    if let Some(section) = file_write_policy {
        policy_sections.push(section);
    }
    if let Some(section) = file_read_deny_policy {
        policy_sections.push(section);
    }
    if let Some(section) = network_policy {
        policy_sections.push(section);
    }
    let full_policy = policy_sections.join(
        "
",
    );

    let mut seatbelt_args: Vec<String> = vec!["-p".to_string(), full_policy];
    seatbelt_args.extend(extra_cli_args);
    seatbelt_args.push("--".to_string());
    seatbelt_args.extend(command);
    seatbelt_args
}

#[cfg(test)]
mod tests {
    use super::MACOS_SEATBELT_BASE_POLICY;
    use super::create_seatbelt_command_args;
    use crate::protocol::SandboxPolicy;
    use crate::sensitive_paths::SensitivePathConfig;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn create_seatbelt_args_with_read_only_git_subpath() {
        if cfg!(target_os = "windows") {
            // /tmp does not exist on Windows, so skip this test.
            return;
        }

        // Create a temporary workspace with two writable roots: one containing
        // a top-level .git directory and one without it.
        let tmp = TempDir::new().expect("tempdir");
        let PopulatedTmp {
            root_with_git,
            root_without_git,
            root_with_git_canon,
            root_with_git_git_canon,
            root_without_git_canon,
        } = populate_tmpdir(tmp.path());
        let cwd = tmp.path().join("cwd");

        // Build a policy that only includes the two test roots as writable and
        // does not automatically include defaults TMPDIR or /tmp.
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![root_with_git, root_without_git],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };

        let args = create_seatbelt_command_args(
            vec!["/bin/echo".to_string(), "hello".to_string()],
            &policy,
            &cwd,
            &SensitivePathConfig::default(),
        );

        // Build the expected policy text using a raw string for readability.
        // Note that the policy includes:
        // - the base policy,
        // - read-only access to the filesystem,
        // - write access to WRITABLE_ROOT_0 (but not its .git) and WRITABLE_ROOT_1.
        let expected_policy = format!(
            r#"{MACOS_SEATBELT_BASE_POLICY}
; allow read-only file operations
(allow file-read*)
(allow file-write*
(require-all (subpath (param "WRITABLE_ROOT_0")) (require-not (subpath (param "WRITABLE_ROOT_0_RO_0"))) ) (subpath (param "WRITABLE_ROOT_1")) (subpath (param "WRITABLE_ROOT_2"))
)"#,
        );

        let mut expected_args = vec![
            "-p".to_string(),
            expected_policy,
            format!(
                "-DWRITABLE_ROOT_0={}",
                root_with_git_canon.to_string_lossy()
            ),
            format!(
                "-DWRITABLE_ROOT_0_RO_0={}",
                root_with_git_git_canon.to_string_lossy()
            ),
            format!(
                "-DWRITABLE_ROOT_1={}",
                root_without_git_canon.to_string_lossy()
            ),
            format!("-DWRITABLE_ROOT_2={}", cwd.to_string_lossy()),
        ];

        expected_args.extend(vec![
            "--".to_string(),
            "/bin/echo".to_string(),
            "hello".to_string(),
        ]);

        assert_eq!(expected_args, args);
    }

    #[test]
    fn create_seatbelt_args_for_cwd_as_git_repo() {
        if cfg!(target_os = "windows") {
            // /tmp does not exist on Windows, so skip this test.
            return;
        }

        // Create a temporary workspace with two writable roots: one containing
        // a top-level .git directory and one without it.
        let tmp = TempDir::new().expect("tempdir");
        let PopulatedTmp {
            root_with_git,
            root_with_git_canon,
            root_with_git_git_canon,
            ..
        } = populate_tmpdir(tmp.path());

        // Build a policy that does not specify any writable_roots, but does
        // use the default ones (cwd and TMPDIR) and verifies the `.git` check
        // is done properly for cwd.
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        };

        let args = create_seatbelt_command_args(
            vec!["/bin/echo".to_string(), "hello".to_string()],
            &policy,
            root_with_git.as_path(),
            &SensitivePathConfig::default(),
        );

        let tmpdir_env_var = std::env::var("TMPDIR")
            .ok()
            .map(PathBuf::from)
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.to_string_lossy().to_string());

        let tempdir_policy_entry = if tmpdir_env_var.is_some() {
            r#" (subpath (param "WRITABLE_ROOT_2"))"#
        } else {
            ""
        };

        // Build the expected policy text using a raw string for readability.
        // Note that the policy includes:
        // - the base policy,
        // - read-only access to the filesystem,
        // - write access to WRITABLE_ROOT_0 (but not its .git) and WRITABLE_ROOT_1.
        let expected_policy = format!(
            r#"{MACOS_SEATBELT_BASE_POLICY}
; allow read-only file operations
(allow file-read*)
(allow file-write*
(require-all (subpath (param "WRITABLE_ROOT_0")) (require-not (subpath (param "WRITABLE_ROOT_0_RO_0"))) ) (subpath (param "WRITABLE_ROOT_1")){tempdir_policy_entry}
)"#,
        );

        let mut expected_args = vec![
            "-p".to_string(),
            expected_policy,
            format!(
                "-DWRITABLE_ROOT_0={}",
                root_with_git_canon.to_string_lossy()
            ),
            format!(
                "-DWRITABLE_ROOT_0_RO_0={}",
                root_with_git_git_canon.to_string_lossy()
            ),
            format!(
                "-DWRITABLE_ROOT_1={}",
                PathBuf::from("/tmp")
                    .canonicalize()
                    .expect("canonicalize /tmp")
                    .to_string_lossy()
            ),
        ];

        if let Some(p) = tmpdir_env_var {
            expected_args.push(format!("-DWRITABLE_ROOT_2={p}"));
        }

        expected_args.extend(vec![
            "--".to_string(),
            "/bin/echo".to_string(),
            "hello".to_string(),
        ]);

        assert_eq!(expected_args, args);
    }

    #[test]
    fn create_seatbelt_args_include_sensitive_read_denies() {
        if cfg!(target_os = "windows") {
            // Seatbelt is macOS-only; skip on Windows builders.
            return;
        }

        let tmp = TempDir::new().expect("tempdir");
        let sandbox_cwd = tmp.path();
        let sensitive_file = sandbox_cwd.join(".env.local");
        std::fs::write(&sensitive_file, "secret").expect("create .env.local");
        let allowed_file = sandbox_cwd.join(".env.example");
        std::fs::write(&allowed_file, "example").expect("create .env.example");

        let args = create_seatbelt_command_args(
            vec!["/bin/echo".to_string()],
            &SandboxPolicy::ReadOnly,
            sandbox_cwd,
            &SensitivePathConfig::default(),
        );

        let sensitive_canon = sensitive_file
            .canonicalize()
            .expect("canonicalize sensitive file");

        let expected_policy = format!(
            r#"{MACOS_SEATBELT_BASE_POLICY}
; allow read-only file operations
(allow file-read*)
(deny file-read*
    (path (param "SENSITIVE_DENY_0"))
    (path (param "SENSITIVE_DENY_1"))
    (path (param "SENSITIVE_DENY_2"))
)"#,
        );

        let expected_args = vec![
            "-p".to_string(),
            expected_policy,
            format!("-DSENSITIVE_DENY_0={}", sensitive_canon.to_string_lossy()),
            "-DSENSITIVE_DENY_1=.env.local".to_string(),
            "-DSENSITIVE_DENY_2=./.env.local".to_string(),
            "--".to_string(),
            "/bin/echo".to_string(),
        ];

        assert_eq!(expected_args, args);
    }

    struct PopulatedTmp {
        root_with_git: PathBuf,
        root_without_git: PathBuf,
        root_with_git_canon: PathBuf,
        root_with_git_git_canon: PathBuf,
        root_without_git_canon: PathBuf,
    }

    fn populate_tmpdir(tmp: &Path) -> PopulatedTmp {
        let root_with_git = tmp.join("with_git");
        let root_without_git = tmp.join("no_git");
        fs::create_dir_all(&root_with_git).expect("create with_git");
        fs::create_dir_all(&root_without_git).expect("create no_git");
        fs::create_dir_all(root_with_git.join(".git")).expect("create .git");

        // Ensure we have canonical paths for -D parameter matching.
        let root_with_git_canon = root_with_git.canonicalize().expect("canonicalize with_git");
        let root_with_git_git_canon = root_with_git_canon.join(".git");
        let root_without_git_canon = root_without_git
            .canonicalize()
            .expect("canonicalize no_git");
        PopulatedTmp {
            root_with_git,
            root_without_git,
            root_with_git_canon,
            root_with_git_git_canon,
            root_without_git_canon,
        }
    }
}
