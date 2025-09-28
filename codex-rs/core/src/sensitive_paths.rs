use std::path::Path;

use glob::glob;
use serde::Deserialize;
use tracing::warn;
use wildmatch::WildMatchPattern;

use std::collections::BTreeSet;
use std::path::PathBuf;

type PathPattern = WildMatchPattern<'*', '?'>;

fn normalize_candidate(value: &str) -> String {
    value.replace('\\', "/")
}

fn is_path_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')
}

fn normalize_path(path: &Path) -> (String, Option<String>) {
    let path_lossy = path.to_string_lossy();
    let path_str = normalize_candidate(&path_lossy);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalize_candidate);
    (path_str, file_name)
}

fn compile_patterns(patterns: &[String]) -> Vec<PathPattern> {
    patterns
        .iter()
        .map(String::as_str)
        .map(WildMatchPattern::new)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct SensitivePathsToml {
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensitivePathConfig {
    deny: Vec<PathPattern>,
    deny_raw: Vec<String>,
    allow: Vec<PathPattern>,
    allow_raw: Vec<String>,
}

impl Default for SensitivePathConfig {
    fn default() -> Self {
        Self::from_lists(
            vec![".env".to_string(), ".env.*".to_string()],
            vec![".env.example".to_string()],
        )
    }
}

impl SensitivePathConfig {
    pub fn from_toml(toml: Option<SensitivePathsToml>) -> Self {
        let mut deny_patterns = vec![".env".to_string(), ".env.*".to_string()];
        let mut allow_patterns = vec![".env.example".to_string()];

        if let Some(toml) = toml {
            deny_patterns.extend(toml.deny);
            allow_patterns.extend(toml.allow);
        }

        Self::from_lists(deny_patterns, allow_patterns)
    }

    fn from_lists(deny: Vec<String>, allow: Vec<String>) -> Self {
        let (allow, skipped_allows): (Vec<String>, Vec<String>) = allow
            .into_iter()
            .partition(|candidate| !is_absolute_pattern(candidate));

        for skipped in skipped_allows {
            warn!("ignoring absolute sensitive-path allow entry: {skipped}");
        }

        Self {
            deny: compile_patterns(&deny),
            deny_raw: deny,
            allow: compile_patterns(&allow),
            allow_raw: allow,
        }
    }

    pub fn deny_patterns(&self) -> &[String] {
        &self.deny_raw
    }

    pub fn is_path_sensitive(&self, path: &Path) -> bool {
        let (normalized, file_name) = normalize_path(path);
        self.matches(&normalized, file_name.as_deref())
    }

    pub fn is_candidate_sensitive(&self, candidate: &str) -> bool {
        let normalized = normalize_candidate(candidate);
        let file_name = Path::new(&normalized)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string);
        if self.matches(&normalized, file_name.as_deref()) {
            return true;
        }

        for token in normalized
            .split(|c: char| !is_path_token_char(c))
            .filter(|s| !s.is_empty())
        {
            if self.matches(token, Some(token)) {
                return true;
            }
        }

        false
    }

    fn matches(&self, path: &str, file_name: Option<&str>) -> bool {
        if self.is_allowed(path, file_name) {
            return false;
        }
        self.deny.iter().any(|pattern| {
            pattern.matches(path) || file_name.is_some_and(|name| pattern.matches(name))
        })
    }

    fn is_allowed(&self, path: &str, file_name: Option<&str>) -> bool {
        self.allow.iter().any(|pattern| {
            pattern.matches(path) || file_name.is_some_and(|name| pattern.matches(name))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedSensitivePath {
    pub absolute: PathBuf,
    pub canonical: PathBuf,
    pub relative: Option<PathBuf>,
}

impl ResolvedSensitivePath {
    pub fn variants(&self) -> Vec<PathBuf> {
        let mut variants = Vec::new();
        variants.push(self.absolute.clone());
        if self.absolute != self.canonical {
            variants.push(self.canonical.clone());
        }
        if let Some(relative) = &self.relative {
            variants.push(relative.clone());
            variants.push(PathBuf::from(".").join(relative));
        }
        variants
    }
}

impl SensitivePathConfig {
    pub fn resolve_paths(&self, sandbox_policy_cwd: &Path) -> Vec<ResolvedSensitivePath> {
        let sandbox_policy_cwd = if sandbox_policy_cwd.is_absolute() {
            sandbox_policy_cwd.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(sandbox_policy_cwd)
        };

        let sandbox_policy_cwd = sandbox_policy_cwd
            .canonicalize()
            .unwrap_or(sandbox_policy_cwd);

        let mut results: Vec<ResolvedSensitivePath> = Vec::new();
        let mut seen: BTreeSet<(PathBuf, PathBuf, Option<PathBuf>)> = BTreeSet::new();

        for pattern in &self.deny_raw {
            let absolute_pattern = if Path::new(pattern).is_absolute() {
                pattern.clone()
            } else {
                sandbox_policy_cwd
                    .join(pattern)
                    .to_string_lossy()
                    .into_owned()
            };

            match glob(&absolute_pattern) {
                Ok(paths) => {
                    for entry in paths.flatten() {
                        let canonical = entry.canonicalize().unwrap_or(entry.clone());
                        if !self.is_path_sensitive(&canonical) {
                            continue;
                        }

                        let relative = canonical
                            .strip_prefix(&sandbox_policy_cwd)
                            .ok()
                            .map(PathBuf::from);

                        let key = (entry.clone(), canonical.clone(), relative.clone());
                        if seen.insert(key) {
                            results.push(ResolvedSensitivePath {
                                absolute: entry,
                                canonical,
                                relative,
                            });
                        }
                    }
                }
                Err(_) => {
                    // Ignore malformed patterns; runtime checks still protect us.
                }
            }
        }

        results.sort();
        results
    }
}

fn is_absolute_pattern(candidate: &str) -> bool {
    if candidate.starts_with('/') {
        return true;
    }

    if candidate.starts_with("\\\\") {
        return true;
    }

    if candidate.len() >= 2
        && candidate.as_bytes()[1] == b':'
        && candidate.as_bytes()[0].is_ascii_alphabetic()
    {
        return true;
    }

    if let Some(stripped) = candidate.strip_prefix('~') {
        return stripped.starts_with('/') || stripped.is_empty();
    }

    Path::new(candidate).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn default_blocks_env_allows_example() {
        let config = SensitivePathConfig::default();
        assert!(config.is_path_sensitive(Path::new(".env")));
        assert!(config.is_path_sensitive(Path::new("sub/.env.local")));
        assert!(!config.is_path_sensitive(Path::new(".env.example")));
    }

    #[test]
    fn allow_pattern_overrides_deny() {
        let config = SensitivePathConfig::from_toml(Some(SensitivePathsToml {
            deny: vec!["**/secrets.json".to_string()],
            allow: vec!["public/secrets.json".to_string()],
        }));

        assert!(config.is_path_sensitive(Path::new("foo/secrets.json")));
        assert!(!config.is_path_sensitive(Path::new("public/secrets.json")));
    }

    #[test]
    fn string_candidate_normalized() {
        let config = SensitivePathConfig::default();
        assert!(config.is_candidate_sensitive("directory\\.env"));
        assert!(!config.is_candidate_sensitive("README.md"));
    }

    #[test]
    fn resolve_paths_include_relative_variants() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = tmp.path();
        let file = cwd.join(".env.secret");
        std::fs::write(&file, "secret").expect("create secret file");

        let config = SensitivePathConfig::from_lists(
            vec![".env.secret".to_string()],
            vec![".env.example".to_string()],
        );

        let resolved = config.resolve_paths(cwd);
        assert_eq!(resolved.len(), 1);
        let entry = &resolved[0];
        let canonical = file.canonicalize().unwrap();
        assert_eq!(entry.canonical, canonical);
        assert_eq!(entry.absolute.canonicalize().unwrap(), canonical);
        assert_eq!(
            entry.relative.as_ref().map(PathBuf::from),
            Some(PathBuf::from(".env.secret"))
        );

        let variants: std::collections::BTreeSet<String> = entry
            .variants()
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(variants.contains(&canonical.to_string_lossy().into_owned()));
        assert!(variants.contains(".env.secret"));
        assert!(variants.contains("./.env.secret"));
    }
}
