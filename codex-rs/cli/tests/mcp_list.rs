use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use pretty_assertions::assert_eq;
use serde_json::Value as JsonValue;
use serde_json::json;
use tempfile::TempDir;

const MANAGED_CONFIG_ENV: &str = "CODEX_MANAGED_CONFIG_PATH";

struct EnvVarGuard {
    original: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(value: &std::ffi::OsStr) -> Self {
        let original = std::env::var_os(MANAGED_CONFIG_ENV);
        unsafe {
            std::env::set_var(MANAGED_CONFIG_ENV, value);
        }
        Self { original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(original) = self.original.take() {
                std::env::set_var(MANAGED_CONFIG_ENV, original);
            } else {
                std::env::remove_var(MANAGED_CONFIG_ENV);
            }
        }
    }
}

struct ManagedConfigGuard {
    path: PathBuf,
    _env_guard: EnvVarGuard,
}

impl ManagedConfigGuard {
    fn new(codex_home: &Path) -> Result<Self> {
        let path = codex_home.join("managed_config.toml");
        std::fs::write(
            &path,
            "[managed]\n\
enable_mcp_servers = true\n\
enable_user_mcp_servers = true\n",
        )?;
        let _env_guard = EnvVarGuard::set(path.as_os_str());
        Ok(Self { path, _env_guard })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn codex_command(codex_home: &Path, managed_config_path: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::cargo_bin("codex")?;
    cmd.env("CODEX_HOME", codex_home);
    cmd.env(MANAGED_CONFIG_ENV, managed_config_path);
    Ok(cmd)
}

#[test]
fn list_shows_empty_state() -> Result<()> {
    let codex_home = TempDir::new()?;
    let managed_config = ManagedConfigGuard::new(codex_home.path())?;

    let mut cmd = codex_command(codex_home.path(), managed_config.path())?;
    let output = cmd.args(["mcp", "list"]).output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("No MCP servers currently active."));

    Ok(())
}

#[test]
fn list_and_get_render_expected_output() -> Result<()> {
    let codex_home = TempDir::new()?;
    let managed_config = ManagedConfigGuard::new(codex_home.path())?;

    let mut add = codex_command(codex_home.path(), managed_config.path())?;
    add.args([
        "mcp",
        "add",
        "docs",
        "--env",
        "TOKEN=secret",
        "--",
        "docs-server",
        "--port",
        "4000",
    ])
    .assert()
    .success();

    let mut list_cmd = codex_command(codex_home.path(), managed_config.path())?;
    let list_output = list_cmd.args(["mcp", "list"]).output()?;
    assert!(list_output.status.success());
    let stdout = String::from_utf8(list_output.stdout)?;
    assert!(stdout.contains("Name"));
    assert!(stdout.contains("docs"));
    assert!(stdout.contains("docs-server"));
    assert!(stdout.contains("TOKEN=secret"));
    assert!(stdout.contains("Status"));
    assert!(stdout.contains("Auth"));
    assert!(stdout.contains("Source"));
    assert!(stdout.contains("enabled"));
    assert!(stdout.contains("Unsupported"));

    let mut list_json_cmd = codex_command(codex_home.path(), managed_config.path())?;
    let json_output = list_json_cmd.args(["mcp", "list", "--json"]).output()?;
    assert!(json_output.status.success());
    let stdout = String::from_utf8(json_output.stdout)?;
    let parsed: JsonValue = serde_json::from_str(&stdout)?;
    assert_eq!(
        parsed,
        json!([
          {
            "name": "docs",
            "enabled": true,
            "transport": {
              "type": "stdio",
              "command": "docs-server",
              "args": [
                "--port",
                "4000"
              ],
              "env": {
                "TOKEN": "secret"
              }
            },
            "startup_timeout_sec": null,
            "tool_timeout_sec": null,
            "auth_status": "unsupported",
            "source": "user"
          }
        ]
        )
    );

    let mut get_cmd = codex_command(codex_home.path(), managed_config.path())?;
    let get_output = get_cmd.args(["mcp", "get", "docs"]).output()?;
    assert!(get_output.status.success());
    let stdout = String::from_utf8(get_output.stdout)?;
    assert!(stdout.contains("docs"));
    assert!(stdout.contains("transport: stdio"));
    assert!(stdout.contains("command: docs-server"));
    assert!(stdout.contains("args: --port 4000"));
    assert!(stdout.contains("env: TOKEN=secret"));
    assert!(stdout.contains("source: user"));
    assert!(stdout.contains("enabled: true"));
    assert!(stdout.contains("remove: codex mcp remove docs"));

    let get_json_output = codex_command(codex_home.path(), managed_config.path())?
        .args(["mcp", "get", "docs", "--json"])
        .output()?;
    assert!(get_json_output.status.success());
    let stdout = String::from_utf8(get_json_output.stdout)?;
    let parsed: JsonValue = serde_json::from_str(&stdout)?;
    assert_eq!(parsed["source"], json!("user"));

    Ok(())
}
