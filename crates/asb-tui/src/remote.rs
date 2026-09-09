// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Safe SSH host discovery and command planning for the remote runner path.
//!
//! This module deliberately does not implement a second SSH client.  OpenSSH remains
//! authoritative for `Include`, `Match`, proxying, host-key verification, agents, and
//! multiplexing.  ASB only parses concrete aliases for display and supplies a fixed
//! argv vector to `ssh`; no user text is ever interpolated into a shell command.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Maximum config bytes read by one discovery operation.
pub const MAX_CONFIG_BYTES: usize = 1_048_576;
/// Maximum include depth accepted during bounded discovery.
pub const MAX_INCLUDE_DEPTH: usize = 8;
/// Fixed runner-side probe command.  It is passed as separate argv values.
pub const REMOTE_PROXY_PROBE: &[&str] = &["asb", "remote-proxy", "--stdio", "--probe"];
const RESTRICTED_KEY_INSTALL_SCRIPT: &str = r#"set -eu
umask 077
directory="$HOME/.ssh"
authorized="$directory/authorized_keys"
mkdir -p "$directory"
chmod 700 "$directory"
touch "$authorized"
chmod 600 "$authorized"
key=$(cat)
case "$key" in
  "ssh-ed25519 "*) ;;
  *) exit 64 ;;
esac
line="restrict,command=\"asb remote-proxy --stdio\" $key"
if ! grep -F -x -- "$line" "$authorized" >/dev/null 2>&1; then
  temporary=$(mktemp "$directory/authorized_keys.asb.XXXXXX")
  chmod 600 "$temporary"
  cat "$authorized" >"$temporary"
  printf '%s\n' "$line" >>"$temporary"
  mv -f -- "$temporary" "$authorized"
fi
"#;

/// A concrete, non-pattern OpenSSH host alias shown to the user.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SshHostProfile {
    /// Alias as written in a `Host` directive.
    pub alias: String,
    /// Config file that declared the alias.
    pub source: PathBuf,
}

/// A redacted command plan that can be executed without a shell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshCommandPlan {
    /// Executable (`ssh`, never a user-supplied path).
    pub program: String,
    /// Exact argv passed to the executable.
    pub args: Vec<String>,
}

/// Explicit, confirmation-gated restricted-key enrollment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyEnrollmentPlan {
    /// Fixed OpenSSH invocation and remote script.
    pub command: SshCommandPlan,
    /// Public key payload; private key material is never read.
    pub public_key: Vec<u8>,
    /// Display-only fingerprint supplied by the caller after independent inspection.
    pub fingerprint: String,
}

impl KeyEnrollmentPlan {
    /// Prepare a non-destructive restricted `authorized_keys` update.
    pub fn prepare(
        profile: &SshHostProfile,
        public_key_path: &Path,
        fingerprint: &str,
        confirmation: &str,
    ) -> Result<Self, RemoteError> {
        validate_alias(&profile.alias)?;
        if fingerprint.is_empty()
            || fingerprint.len() > 255
            || fingerprint
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || confirmation != format!("install {} {}", profile.alias, fingerprint)
        {
            return Err(RemoteError::ConfirmationRequired);
        }
        let metadata = fs::metadata(public_key_path).map_err(RemoteError::Io)?;
        if !metadata.is_file() || metadata.len() > 8192 {
            return Err(RemoteError::InvalidPublicKey);
        }
        let mut public_key = fs::read(public_key_path).map_err(RemoteError::Io)?;
        if public_key.last() == Some(&b'\n') {
            public_key.pop();
        }
        if public_key.len() < 12
            || public_key.contains(&b'\n')
            || public_key.contains(&b'\r')
            || !public_key.starts_with(b"ssh-ed25519 ")
        {
            return Err(RemoteError::InvalidPublicKey);
        }
        let mut args = vec!["-T".to_owned(), "--".to_owned(), profile.alias.clone()];
        args.extend([
            "sh".to_owned(),
            "-c".to_owned(),
            RESTRICTED_KEY_INSTALL_SCRIPT.to_owned(),
        ]);
        Ok(Self {
            command: SshCommandPlan {
                program: "ssh".to_owned(),
                args,
            },
            public_key,
            fingerprint: fingerprint.to_owned(),
        })
    }

    /// Spawn enrollment and send only the public key over stdin.
    pub fn spawn(&self) -> io::Result<Child> {
        let mut child = self.command.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(&self.public_key)?;
        }
        Ok(child)
    }
}

impl SshCommandPlan {
    /// Build a non-interactive stdio bridge while retaining OpenSSH host-key checks.
    pub fn for_profile(profile: &SshHostProfile) -> Result<Self, RemoteError> {
        validate_alias(&profile.alias)?;
        let mut args = vec!["-T".to_owned(), "--".to_owned(), profile.alias.clone()];
        args.extend(REMOTE_PROXY_PROBE.iter().map(|part| (*part).to_owned()));
        Ok(Self {
            program: "ssh".to_owned(),
            args,
        })
    }

    /// Ask the installed OpenSSH client for its fully resolved configuration.
    /// OpenSSH remains authoritative for `Include`, `Match`, proxying, agents,
    /// canonicalization, and multiplexing.
    pub fn config_query(profile: &SshHostProfile) -> Result<Self, RemoteError> {
        validate_alias(&profile.alias)?;
        Ok(Self {
            program: "ssh".to_owned(),
            args: vec!["-G".to_owned(), "--".to_owned(), profile.alias.clone()],
        })
    }

    /// Spawn OpenSSH with isolated stdio and no shell interpolation.
    pub fn spawn(&self) -> io::Result<Child> {
        Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
}

/// A successful connection recorded without host identity or private details.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecentConnection {
    /// Concrete OpenSSH alias.
    pub alias: String,
    /// Unix seconds of the last successful ASB probe.
    pub last_success_epoch: u64,
}

/// Bounded recent-runner ledger.  The ledger is not a replacement for SSH config.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentConnectionLedger {
    /// Schema version.
    pub schema_version: u16,
    /// Most-recent successful aliases, newest first after normalization.
    pub connections: Vec<RecentConnection>,
}

impl RecentConnectionLedger {
    /// Ledger schema version.
    pub const VERSION: u16 = 1;

    /// Read a mode-0600 ledger and reject malformed or oversized state.
    pub fn read(path: &Path) -> Result<Self, RemoteError> {
        let metadata = fs::metadata(path).map_err(RemoteError::Io)?;
        if metadata.len() > MAX_CONFIG_BYTES as u64 {
            return Err(RemoteError::TooLarge);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(RemoteError::InsecurePermissions);
            }
        }
        let value: Self = serde_json::from_slice(&fs::read(path).map_err(RemoteError::Io)?)
            .map_err(|_| RemoteError::MalformedLedger)?;
        value.validate()?;
        Ok(value)
    }

    /// Validate and atomically replace a mode-0600 ledger.
    pub fn write(&self, path: &Path) -> Result<(), RemoteError> {
        self.validate()?;
        let parent = path.parent().ok_or(RemoteError::InvalidPath)?;
        fs::create_dir_all(parent).map_err(RemoteError::Io)?;
        let tmp = path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| RemoteError::MalformedLedger)?;
        fs::write(&tmp, bytes).map_err(RemoteError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
                .map_err(RemoteError::Io)?;
        }
        fs::rename(tmp, path).map_err(RemoteError::Io)
    }

    /// Record a successful alias and retain at most 256 entries.
    pub fn record_success(&mut self, alias: &str, epoch: u64) -> Result<(), RemoteError> {
        validate_alias(alias)?;
        self.connections.retain(|entry| entry.alias != alias);
        self.connections.push(RecentConnection {
            alias: alias.to_owned(),
            last_success_epoch: epoch,
        });
        self.connections
            .sort_by(|left, right| right.last_success_epoch.cmp(&left.last_success_epoch));
        self.connections.truncate(256);
        Ok(())
    }

    fn validate(&self) -> Result<(), RemoteError> {
        if self.schema_version != Self::VERSION || self.connections.len() > 256 {
            return Err(RemoteError::MalformedLedger);
        }
        let mut seen = BTreeMap::new();
        for entry in &self.connections {
            validate_alias(&entry.alias)?;
            if seen.insert(entry.alias.clone(), ()).is_some() {
                return Err(RemoteError::MalformedLedger);
            }
        }
        Ok(())
    }
}

/// Discover concrete aliases from a config and its bounded `Include` files.
pub fn discover_profiles(config: &Path) -> Result<Vec<SshHostProfile>, RemoteError> {
    let mut output = BTreeMap::new();
    discover_file(config, 0, &mut output)?;
    Ok(output
        .into_iter()
        .map(|(alias, source)| SshHostProfile { alias, source })
        .collect())
}

fn discover_file(
    path: &Path,
    depth: usize,
    output: &mut BTreeMap<String, PathBuf>,
) -> Result<(), RemoteError> {
    if depth > MAX_INCLUDE_DEPTH {
        return Err(RemoteError::IncludeDepth);
    }
    let metadata = fs::metadata(path).map_err(RemoteError::Io)?;
    if metadata.len() > MAX_CONFIG_BYTES as u64 {
        return Err(RemoteError::TooLarge);
    }
    let text = fs::read_to_string(path).map_err(RemoteError::Io)?;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let keyword = fields.next().unwrap_or_default();
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                for alias in fields {
                    if !alias.contains(['*', '?', '!']) {
                        validate_alias(alias)?;
                        output
                            .entry(alias.to_owned())
                            .or_insert_with(|| path.to_owned());
                    }
                }
            }
            "include" => {
                for pattern in fields {
                    if pattern.contains(['*', '?']) {
                        let (relative_parent, file_pattern) = pattern
                            .rsplit_once('/')
                            .map_or(("", pattern), |(parent, name)| (parent, name));
                        let parent = path
                            .parent()
                            .unwrap_or_else(|| Path::new("."))
                            .join(relative_parent);
                        for entry in fs::read_dir(parent).map_err(RemoteError::Io)? {
                            let entry = entry.map_err(RemoteError::Io)?;
                            if entry.file_type().map_err(RemoteError::Io)?.is_file() {
                                let name = entry.file_name().to_string_lossy().into_owned();
                                if wildcard_match(file_pattern, &name) {
                                    discover_file(&entry.path(), depth + 1, output)?;
                                }
                            }
                        }
                    } else {
                        let include = Path::new(pattern);
                        let include = if include.is_absolute() {
                            include.to_owned()
                        } else {
                            path.parent()
                                .unwrap_or_else(|| Path::new("."))
                                .join(include)
                        };
                        discover_file(&include, depth + 1, output)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        if pattern.is_empty() {
            return value.is_empty();
        }
        if pattern[0] == b'*' {
            return matches(&pattern[1..], value)
                || (!value.is_empty() && matches(pattern, &value[1..]));
        }
        !value.is_empty()
            && (pattern[0] == b'?' || pattern[0] == value[0])
            && matches(&pattern[1..], &value[1..])
    }
    matches(pattern.as_bytes(), value.as_bytes())
}

fn validate_alias(alias: &str) -> Result<(), RemoteError> {
    if alias.is_empty()
        || alias.len() > 255
        || alias.starts_with('-')
        || alias.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(RemoteError::InvalidAlias);
    }
    Ok(())
}

/// Errors from bounded SSH discovery and command planning.
#[derive(Debug)]
pub enum RemoteError {
    /// Underlying filesystem failure.
    Io(io::Error),
    /// Input exceeded a bounded size.
    TooLarge,
    /// Include nesting exceeded the safety bound.
    IncludeDepth,
    /// Ledger or JSON content was malformed.
    MalformedLedger,
    /// A path did not have a parent.
    InvalidPath,
    /// A ledger was group/world accessible.
    InsecurePermissions,
    /// An alias was not safe to pass as one argv item.
    InvalidAlias,
    /// A key enrollment was not explicitly confirmed.
    ConfirmationRequired,
    /// Public key was absent, oversized, or not a single Ed25519 line.
    InvalidPublicKey,
}

impl PartialEq for RemoteError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
impl Eq for RemoteError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("asb-ssh-{nonce}"));
        fs::create_dir_all(&path).expect("mkdir");
        path
    }

    #[test]
    fn discovers_concrete_hosts_and_includes_without_patterns() {
        let root = temp_dir();
        fs::write(
            root.join("config"),
            "Host alpha *pattern\n  Include conf.d/*\n",
        )
        .expect("config");
        fs::create_dir(root.join("conf.d")).expect("include dir");
        fs::write(root.join("conf.d/one"), "Host beta\nHost !neg\n").expect("include");
        let profiles = discover_profiles(&root.join("config")).expect("profiles");
        assert_eq!(
            profiles
                .iter()
                .map(|p| p.alias.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn command_plan_is_fixed_argv_and_rejects_option_injection() {
        let profile = SshHostProfile {
            alias: "runner".into(),
            source: PathBuf::from("config"),
        };
        assert_eq!(
            SshCommandPlan::for_profile(&profile).expect("plan").args,
            [
                "-T",
                "--",
                "runner",
                "asb",
                "remote-proxy",
                "--stdio",
                "--probe"
            ]
        );
        let bad = SshHostProfile {
            alias: "-oProxyCommand=bad".into(),
            source: PathBuf::new(),
        };
        assert_eq!(
            SshCommandPlan::for_profile(&bad),
            Err(RemoteError::InvalidAlias)
        );
        assert_eq!(
            SshCommandPlan::config_query(&profile)
                .expect("config query")
                .args,
            ["-G", "--", "runner"]
        );
    }

    #[test]
    fn key_enrollment_requires_confirmation_and_never_reads_private_keys() {
        let root = temp_dir();
        let public = root.join("id_ed25519.pub");
        fs::write(
            &public,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIexample comment\n",
        )
        .expect("public key");
        let profile = SshHostProfile {
            alias: "runner".into(),
            source: root.join("config"),
        };
        assert_eq!(
            KeyEnrollmentPlan::prepare(&profile, &public, "SHA256:fingerprint", "no"),
            Err(RemoteError::ConfirmationRequired)
        );
        let plan = KeyEnrollmentPlan::prepare(
            &profile,
            &public,
            "SHA256:fingerprint",
            "install runner SHA256:fingerprint",
        )
        .expect("enrollment");
        assert_eq!(
            plan.public_key,
            b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIexample comment"
        );
        assert!(
            plan.command
                .args
                .iter()
                .any(|arg| arg.contains("restrict,command"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ledger_normalizes_recency_and_rejects_duplicate_state() {
        let mut ledger = RecentConnectionLedger {
            schema_version: 1,
            connections: Vec::new(),
        };
        ledger.record_success("old", 1).expect("old");
        ledger.record_success("new", 2).expect("new");
        ledger.record_success("old", 3).expect("refresh");
        assert_eq!(ledger.connections[0].alias, "old");
        let duplicate = RecentConnectionLedger {
            schema_version: 1,
            connections: vec![
                RecentConnection {
                    alias: "x".into(),
                    last_success_epoch: 1,
                },
                RecentConnection {
                    alias: "x".into(),
                    last_success_epoch: 2,
                },
            ],
        };
        assert_eq!(duplicate.validate(), Err(RemoteError::MalformedLedger));
    }

    #[test]
    fn ledger_round_trip_is_private_and_atomic() {
        let root = temp_dir();
        let path = root.join("state").join("recent.json");
        let ledger = RecentConnectionLedger {
            schema_version: 1,
            connections: vec![RecentConnection {
                alias: "runner".into(),
                last_success_epoch: 7,
            }],
        };
        ledger.write(&path).expect("write");
        assert_eq!(RecentConnectionLedger::read(&path).expect("read"), ledger);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(root);
    }
}
