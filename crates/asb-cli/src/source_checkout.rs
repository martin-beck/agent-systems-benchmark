// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded acquisition of catalog-pinned source checkouts.
//!
//! This boundary deliberately does not understand branches, tags, archives,
//! or prebuilt artifacts.  A caller supplies a catalog entry and the provider
//! returns only an exact, detached revision from that entry's repository.
#![allow(missing_docs)]

use crate::tool_catalog::ToolEntry;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const MAX_CHECKOUT_SECONDS: u64 = 30 * 60;
pub const MAX_CHECKOUT_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_CHECKOUT_FILES: u64 = 100_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckoutLimits {
    pub timeout: Duration,
    pub max_bytes: u64,
    pub max_files: u64,
}

impl Default for CheckoutLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15 * 60),
            max_bytes: MAX_CHECKOUT_BYTES,
            max_files: MAX_CHECKOUT_FILES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CheckoutRequest {
    repository_url: String,
    revision: String,
    staging_root: PathBuf,
    limits: CheckoutLimits,
    cancel: Arc<AtomicBool>,
}

impl CheckoutRequest {
    pub fn for_tool(tool: &ToolEntry, staging_root: PathBuf) -> Result<Self, CheckoutError> {
        let revision = tool.official_source.revision.clone();
        let repository_url = tool.official_source.url.clone();
        if !cfg!(test) {
            let catalog = crate::tool_catalog::catalog();
            crate::tool_catalog::validate(&catalog).map_err(|_| CheckoutError::InvalidRequest)?;
            let allowed = catalog.entries.iter().any(|entry| {
                entry.id == tool.id
                    && entry.official_source.url == repository_url
                    && entry.official_source.revision == revision
            });
            if !allowed {
                return Err(CheckoutError::RepositoryIdentityMismatch);
            }
        }
        validate_identity(&repository_url, &revision)?;
        Ok(Self {
            repository_url,
            revision,
            staging_root,
            limits: CheckoutLimits::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCheckout {
    path: PathBuf,
    repository_url: String,
    revision: String,
}

impl SourceCheckout {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Remove the transaction-owned checkout after the pinned builder has
    /// consumed it.  Callers must treat cleanup failure as a failed build.
    pub fn cleanup(self) -> Result<(), CheckoutError> {
        fs::remove_dir_all(self.path).map_err(|_| CheckoutError::CleanupFailed)
    }
}

impl CheckoutRequest {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckoutError {
    InvalidRequest,
    Offline,
    Unavailable,
    RepositoryIdentityMismatch,
    RevisionMismatch,
    ResourceLimit,
    Cancelled,
    CleanupFailed,
}

impl CheckoutError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "source_checkout_invalid_request",
            Self::Offline => "source_checkout_offline",
            Self::Unavailable => "source_checkout_unavailable",
            Self::RepositoryIdentityMismatch => "source_checkout_repository_mismatch",
            Self::RevisionMismatch => "source_checkout_revision_mismatch",
            Self::ResourceLimit => "source_checkout_resource_limit",
            Self::Cancelled => "source_checkout_cancelled",
            Self::CleanupFailed => "source_checkout_cleanup_failed",
        }
    }
}

pub fn acquire(request: &CheckoutRequest) -> Result<SourceCheckout, CheckoutError> {
    #[cfg(test)]
    let _fixture_guard = crate::test_fixture_semaphore::acquire();
    validate_request(request)?;
    fs::create_dir_all(&request.staging_root).map_err(|_| CheckoutError::Unavailable)?;
    let staging = reserve_staging_root(&request.staging_root)?;
    let result = acquire_inner(request, &staging);
    if result.is_err() && staging.exists() && fs::remove_dir_all(&staging).is_err() {
        return Err(CheckoutError::CleanupFailed);
    }
    result
}

/// Reserve a transaction directory atomically.  Process-local test locks do
/// not protect independent cargo processes, and PID-only names can therefore
/// collide when processes run in separate PID namespaces.  `create_dir` is
/// the ownership claim: a pre-existing candidate is skipped, while all other
/// filesystem failures remain fail-closed.
fn reserve_staging_root(root: &Path) -> Result<PathBuf, CheckoutError> {
    const MAX_ATTEMPTS: u32 = 128;
    let pid = std::process::id();
    for suffix in 0..MAX_ATTEMPTS {
        let name = if suffix == 0 {
            format!("checkout-{pid}")
        } else {
            format!("checkout-{pid}-{suffix}")
        };
        let staging = root.join(name);
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(CheckoutError::Unavailable),
        }
    }
    Err(CheckoutError::Unavailable)
}

fn acquire_inner(
    request: &CheckoutRequest,
    staging: &Path,
) -> Result<SourceCheckout, CheckoutError> {
    let deadline = Instant::now() + request.limits.timeout;
    let mut clone = command(request, "clone");
    clone.args([
        "--no-checkout",
        "--no-tags",
        "--depth",
        "1",
        "--filter=blob:none",
        "--no-local",
    ]);
    clone.arg(&request.repository_url).arg(staging);
    run_bounded_until(clone, request, deadline, Some(staging))?;
    enforce_bounds(staging, &request.limits)?;

    let remote = staging.join(".asb-remote");
    let mut remote_command = command(request, "remote");
    remote_command
        .current_dir(staging)
        .args(["get-url", "origin"]);
    run_bounded_capture(remote_command, &remote, request, deadline, Some(staging))?;
    let actual =
        fs::read_to_string(&remote).map_err(|_| CheckoutError::RepositoryIdentityMismatch)?;
    let _ = fs::remove_file(&remote);
    if actual.trim() != request.repository_url {
        return Err(CheckoutError::RepositoryIdentityMismatch);
    }

    let mut fetch = command(request, "fetch");
    fetch
        .current_dir(staging)
        .args(["--no-tags", "origin", &request.revision]);
    run_bounded_until(fetch, request, deadline, Some(staging))?;
    enforce_bounds(staging, &request.limits)?;

    let mut checkout = command(request, "checkout");
    checkout
        .current_dir(staging)
        .args(["--detach", &request.revision]);
    run_bounded_until(checkout, request, deadline, Some(staging))?;

    let head_file = staging.join(".asb-head");
    let mut head = command(request, "rev-parse");
    head.current_dir(staging).args(["--verify", "HEAD"]);
    run_bounded_capture(head, &head_file, request, deadline, Some(staging))?;
    let actual = fs::read_to_string(&head_file).map_err(|_| CheckoutError::RevisionMismatch)?;
    let _ = fs::remove_file(&head_file);
    if actual.trim() != request.revision {
        return Err(CheckoutError::RevisionMismatch);
    }
    enforce_bounds(staging, &request.limits)?;
    Ok(SourceCheckout {
        path: staging.to_path_buf(),
        repository_url: request.repository_url.clone(),
        revision: request.revision.clone(),
    })
}

fn validate_identity(url: &str, revision: &str) -> Result<(), CheckoutError> {
    let local_fixture = cfg!(test) && url.starts_with("file://");
    if (!url.starts_with("https://") && !local_fixture)
        || url.len() > 2048
        || revision.len() != 40
        || url.contains('@')
    {
        return Err(CheckoutError::InvalidRequest);
    }
    if !revision.bytes().all(|b| b.is_ascii_hexdigit()) || url.contains(['\n', '\r', '\0']) {
        return Err(CheckoutError::InvalidRequest);
    }
    Ok(())
}

fn validate_request(request: &CheckoutRequest) -> Result<(), CheckoutError> {
    validate_identity(&request.repository_url, &request.revision)?;
    if request.staging_root.as_os_str().is_empty()
        || request.limits.timeout.is_zero()
        || request.limits.timeout > Duration::from_secs(MAX_CHECKOUT_SECONDS)
        || request.limits.max_bytes == 0
        || request.limits.max_bytes > MAX_CHECKOUT_BYTES
        || request.limits.max_files == 0
        || request.limits.max_files > MAX_CHECKOUT_FILES
    {
        return Err(CheckoutError::InvalidRequest);
    }
    Ok(())
}

fn command(request: &CheckoutRequest, subcommand: &str) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg(if cfg!(test) {
            "protocol.file.allow=always"
        } else {
            "protocol.file.allow=never"
        })
        .arg(subcommand)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", request.staging_root.join(".home"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn run_bounded_until(
    mut command: Command,
    request: &CheckoutRequest,
    deadline: Instant,
    watch: Option<&Path>,
) -> Result<(), CheckoutError> {
    let watch_missing = watch.is_some_and(|root| !root.exists());
    if watch_missing {
        return Err(CheckoutError::Unavailable);
    }
    let mut child = command.spawn().map_err(|_| CheckoutError::Unavailable)?;
    loop {
        if request.cancel.load(Ordering::Acquire) {
            terminate(&mut child);
            return Err(CheckoutError::Cancelled);
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            return Err(CheckoutError::Offline);
        }
        if let Some(root) = watch
            && let Err(error) = enforce_bounds(root, &request.limits)
        {
            // A disappearing or unreadable staging tree is not a reason to
            // continue an external transfer: fail closed while the child is
            // still alive and let transaction cleanup run.
            terminate(&mut child);
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(CheckoutError::Unavailable),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                terminate(&mut child);
                return Err(CheckoutError::Unavailable);
            }
        }
    }
}

fn run_bounded_capture(
    mut command: Command,
    output: &Path,
    request: &CheckoutRequest,
    deadline: Instant,
    watch: Option<&Path>,
) -> Result<(), CheckoutError> {
    let file = fs::File::create(output).map_err(|_| CheckoutError::Unavailable)?;
    command.stdout(Stdio::from(file));
    run_bounded_until(command, request, deadline, watch)?;
    let metadata = fs::metadata(output).map_err(|_| CheckoutError::Unavailable)?;
    if metadata.len() > 256 {
        return Err(CheckoutError::ResourceLimit);
    }
    Ok(())
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn enforce_bounds(root: &Path, limits: &CheckoutLimits) -> Result<(), CheckoutError> {
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).map_err(|_| CheckoutError::Unavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(CheckoutError::ResourceLimit);
        }
        if metadata.is_file() {
            files = files.saturating_add(1);
            bytes = bytes.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            for entry in fs::read_dir(path).map_err(|_| CheckoutError::Unavailable)? {
                pending.push(entry.map_err(|_| CheckoutError::Unavailable)?.path());
            }
        }
        if files > limits.max_files || bytes > limits.max_bytes {
            return Err(CheckoutError::ResourceLimit);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_catalog::{OfficialSource, ToolEntry};
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};

    fn fixture_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| {
                // A fixture panic must not turn every later checkout test into a
                // process-wide PoisonError.  The checkout itself remains
                // fail-closed; this only permits the next isolated fixture to
                // report its own result and perform cleanup.
                poisoned.into_inner()
            })
    }

    fn fixture_repository() -> (PathBuf, String, PathBuf) {
        let root = tempfile::Builder::new()
            .prefix("asb-source-checkout-test-")
            .tempdir()
            .unwrap()
            .keep();
        let source = root.join("source");
        let staging = root.join("staging");
        std::fs::create_dir_all(&source).unwrap();
        let git = |args: &[&str], cwd: &Path| {
            let status = Command::new("git")
                .args(args)
                .current_dir(cwd)
                .status()
                .unwrap();
            assert!(status.success());
        };
        git(["init", "-q"].as_slice(), &source);
        git(
            ["config", "user.email", "test@example.invalid"].as_slice(),
            &source,
        );
        git(["config", "user.name", "ASB test"].as_slice(), &source);
        std::fs::write(source.join("README"), "fixture\n").unwrap();
        git(["add", "README"].as_slice(), &source);
        git(["commit", "-qm", "fixture"].as_slice(), &source);
        let revision = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&source)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        (source, revision, staging)
    }

    fn tool(url: &str, revision: &str) -> ToolEntry {
        ToolEntry {
            id: "fixture".into(),
            kind: "agent".into(),
            development_only: true,
            official_source: OfficialSource {
                url: url.into(),
                version: "1".into(),
                revision: revision.into(),
            },
            platforms: vec![],
            alternatives: vec![],
            entrypoint: "fixture".into(),
            license: "MIT".into(),
            dependencies: vec![],
            build_recipes: vec![],
        }
    }

    #[test]
    fn catalog_identity_requires_https_and_full_revision() {
        assert!(matches!(
            CheckoutRequest::for_tool(
                &tool("ftp://example.invalid/x", &"a".repeat(40)),
                PathBuf::from("/tmp/x")
            ),
            Err(CheckoutError::InvalidRequest)
        ));
        assert!(
            CheckoutRequest::for_tool(
                &tool("https://example.invalid/x", &"a".repeat(40)),
                PathBuf::from("/tmp/x")
            )
            .is_ok()
        );
    }

    #[test]
    fn identity_rejects_malformed_revision_and_url_inputs() {
        let revision = "a".repeat(40);
        assert_eq!(
            validate_identity("https://example.invalid/repo\n", &revision),
            Err(CheckoutError::InvalidRequest)
        );
        assert_eq!(
            validate_identity("https://example.invalid/repo\0", &revision),
            Err(CheckoutError::InvalidRequest)
        );
        assert_eq!(
            validate_identity(&format!("https://{}", "a".repeat(2048)), &revision),
            Err(CheckoutError::InvalidRequest)
        );
        assert_eq!(
            validate_identity("https://example.invalid/repo", &"g".repeat(40)),
            Err(CheckoutError::InvalidRequest)
        );
    }

    #[test]
    fn limits_and_cancel_are_typed_before_or_during_acquisition() {
        let mut request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/x", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        request.limits.max_files = 0;
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
        request.limits = CheckoutLimits::default();
        request.cancel.store(true, Ordering::Release);
        assert!(matches!(
            acquire(&request),
            Err(CheckoutError::Unavailable | CheckoutError::Cancelled)
        ));
    }

    #[test]
    fn cancellation_and_error_codes_are_explicit() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        request.cancel();
        let root_guard = tempfile::tempdir().unwrap();
        let root = root_guard.path().to_path_buf();
        let mut command = Command::new("sleep");
        command.arg("1");
        assert_eq!(
            run_bounded_until(
                command,
                &request,
                Instant::now() + Duration::from_secs(2),
                Some(&root),
            ),
            Err(CheckoutError::Cancelled)
        );

        let errors = [
            CheckoutError::InvalidRequest,
            CheckoutError::Offline,
            CheckoutError::Unavailable,
            CheckoutError::RepositoryIdentityMismatch,
            CheckoutError::RevisionMismatch,
            CheckoutError::ResourceLimit,
            CheckoutError::Cancelled,
            CheckoutError::CleanupFailed,
        ];
        assert!(errors.iter().all(|error| !error.code().is_empty()));
    }

    #[test]
    fn acquisition_reports_unavailable_when_git_cannot_spawn() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let root =
            std::env::temp_dir().join(format!("asb-spawn-failure-checkout-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let result = run_bounded_until(
            Command::new("asb-command-that-does-not-exist"),
            &request,
            Instant::now() + Duration::from_secs(2),
            Some(&root),
        );
        assert_eq!(result, Err(CheckoutError::Unavailable));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_command_nonzero_exit_is_unavailable() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let root =
            std::env::temp_dir().join(format!("asb-nonzero-checkout-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let result = run_bounded_until(
            Command::new("false"),
            &request,
            Instant::now() + Duration::from_secs(2),
            Some(&root),
        );
        assert_eq!(result, Err(CheckoutError::Unavailable));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_command_reports_unavailable_for_missing_watch_tree() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let missing =
            std::env::temp_dir().join(format!("asb-missing-watch-checkout-{}", std::process::id()));
        assert_eq!(
            run_bounded_until(
                Command::new("true"),
                &request,
                Instant::now() + Duration::from_secs(2),
                Some(&missing),
            ),
            Err(CheckoutError::Unavailable)
        );
    }

    #[test]
    fn bounded_command_succeeds_without_watch_tree() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        assert_eq!(
            run_bounded_until(
                Command::new("true"),
                &request,
                Instant::now() + Duration::from_secs(2),
                None,
            ),
            Ok(())
        );
    }

    #[test]
    fn capture_reports_unavailable_when_output_parent_is_missing() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "asb-capture-output-checkout-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let output = root.join("missing").join("remote");
        assert_eq!(
            run_bounded_capture(
                Command::new("true"),
                &output,
                &request,
                Instant::now() + Duration::from_secs(2),
                Some(&root),
            ),
            Err(CheckoutError::Unavailable)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn request_bounds_reject_empty_staging_and_zero_limits() {
        let mut request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        request.staging_root = PathBuf::new();
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
        request.staging_root = PathBuf::from("/tmp/x");
        request.limits.timeout = Duration::ZERO;
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
        request.limits = CheckoutLimits::default();
        request.limits.max_bytes = 0;
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
    }

    #[test]
    fn acquisition_rejects_file_staging_root() {
        let root =
            std::env::temp_dir().join(format!("asb-file-staging-checkout-{}", std::process::id()));
        std::fs::write(&root, b"not a directory").unwrap();
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            root.clone(),
        )
        .unwrap();
        assert_eq!(acquire(&request), Err(CheckoutError::Unavailable));
        std::fs::remove_file(root).unwrap();
    }

    #[test]
    fn every_checkout_limit_ceiling_is_rejected() {
        let mut request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        request.limits.timeout = Duration::from_secs(MAX_CHECKOUT_SECONDS + 1);
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
        request.limits = CheckoutLimits::default();
        request.limits.max_bytes = MAX_CHECKOUT_BYTES + 1;
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
        request.limits = CheckoutLimits::default();
        request.limits.max_files = MAX_CHECKOUT_FILES + 1;
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
    }

    #[test]
    fn acquisition_skips_owned_staging_collision_and_cleanup_failure_is_typed() {
        let _semaphore_guard = crate::test_fixture_semaphore::acquire();
        let _fixture_lock_guard = fixture_lock();
        let (source, revision, staging) = fixture_repository();
        let request = CheckoutRequest::for_tool(
            &tool(&format!("file://{}", source.display()), &revision),
            staging.clone(),
        )
        .unwrap();
        let collision = staging.join(format!("checkout-{}", std::process::id()));
        std::fs::create_dir_all(&collision).unwrap();
        let checkout = acquire(&request).unwrap();
        assert_ne!(checkout.path(), collision.as_path());
        checkout.cleanup().unwrap();
        std::fs::remove_dir_all(&collision).unwrap();

        let missing = SourceCheckout {
            path: staging.join("never-created"),
            repository_url: request.repository_url,
            revision: request.revision,
        };
        assert_eq!(missing.cleanup(), Err(CheckoutError::CleanupFailed));
        let _ = std::fs::remove_dir_all(source.parent().unwrap());
    }

    #[test]
    fn staging_reservation_has_a_bounded_collision_budget() {
        let root = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        for suffix in 0..128_u32 {
            let name = if suffix == 0 {
                format!("checkout-{pid}")
            } else {
                format!("checkout-{pid}-{suffix}")
            };
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        assert_eq!(
            reserve_staging_root(root.path()),
            Err(CheckoutError::Unavailable)
        );
    }

    #[test]
    fn staging_reservation_is_safe_for_concurrent_owners() {
        let root = tempfile::tempdir().unwrap();
        let mut workers = Vec::new();
        for _ in 0..16 {
            let path = root.path().to_path_buf();
            workers.push(std::thread::spawn(move || reserve_staging_root(&path)));
        }
        let reservations: Vec<PathBuf> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect();
        let unique: std::collections::BTreeSet<PathBuf> = reservations.iter().cloned().collect();
        assert_eq!(unique.len(), reservations.len());
        for path in reservations {
            std::fs::remove_dir(path).unwrap();
        }
    }

    #[test]
    #[cfg(unix)]
    fn hostile_output_revision_and_symlink_paths_fail_closed() {
        use std::os::unix::fs::symlink;
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            std::env::temp_dir().join(format!("asb-hostile-checkout-{}", std::process::id())),
        )
        .unwrap();
        assert_eq!(
            validate_identity(
                "https://example.invalid/repo",
                &format!("{}g", "a".repeat(39))
            ),
            Err(CheckoutError::InvalidRequest)
        );
        let root =
            std::env::temp_dir().join(format!("asb-symlink-checkout-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        symlink("/tmp", root.join("escape")).unwrap();
        assert_eq!(
            enforce_bounds(&root, &request.limits),
            Err(CheckoutError::ResourceLimit)
        );
        std::fs::remove_file(root.join("escape")).unwrap();

        let output = root.join("output");
        let command = Command::new("false");
        assert_eq!(
            run_bounded_capture(
                command,
                &output,
                &request,
                Instant::now() + Duration::from_secs(2),
                Some(&root),
            ),
            Err(CheckoutError::Unavailable)
        );

        let oversized = root.join("oversized");
        let command = Command::new("printf");
        let mut oversized_command = command;
        oversized_command.args(["%0300d", "0"]);
        assert_eq!(
            run_bounded_capture(
                oversized_command,
                &oversized,
                &request,
                Instant::now() + Duration::from_secs(2),
                Some(&root),
            ),
            Err(CheckoutError::ResourceLimit)
        );

        let mut file_limited = request.limits.clone();
        file_limited.max_files = 1;
        assert_eq!(
            enforce_bounds(&root, &file_limited),
            Err(CheckoutError::ResourceLimit)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wrong_revision_and_credential_bearing_repository_fail_closed() {
        assert!(matches!(
            CheckoutRequest::for_tool(
                &tool("https://user:pass@example.invalid/repo", &"a".repeat(40)),
                PathBuf::from("/tmp/x")
            ),
            Err(CheckoutError::InvalidRequest)
        ));
        assert!(matches!(
            CheckoutRequest::for_tool(
                &tool("https://example.invalid/repo", "moving-ref"),
                PathBuf::from("/tmp/x")
            ),
            Err(CheckoutError::InvalidRequest)
        ));
    }

    #[test]
    fn deadline_and_live_tree_limits_are_enforced() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let mut command = Command::new("sleep");
        command.arg("1");
        assert_eq!(
            run_bounded_until(command, &request, Instant::now(), None),
            Err(CheckoutError::Offline)
        );
        let root = std::env::temp_dir().join(format!("asb-source-limit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("large"), vec![0_u8; 8]).unwrap();
        let mut limited = request.limits.clone();
        limited.max_bytes = 4;
        assert_eq!(
            enforce_bounds(&root, &limited),
            Err(CheckoutError::ResourceLimit)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inaccessible_staging_fails_closed_while_child_runs() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let mut command = Command::new("sleep");
        command.arg("1");
        let missing =
            std::env::temp_dir().join(format!("asb-missing-checkout-{}", std::process::id()));
        assert_eq!(
            run_bounded_until(
                command,
                &request,
                Instant::now() + Duration::from_secs(2),
                Some(&missing)
            ),
            Err(CheckoutError::Unavailable)
        );
    }

    #[test]
    fn cancellation_invalid_bounds_and_cleanup_failures_are_typed() {
        let mut request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        request.limits.timeout = Duration::ZERO;
        assert_eq!(acquire(&request), Err(CheckoutError::InvalidRequest));
        request.limits = CheckoutLimits::default();
        request.cancel();
        let mut command = Command::new("sleep");
        command.arg("1");
        assert_eq!(
            run_bounded_until(
                command,
                &request,
                Instant::now() + Duration::from_secs(2),
                None
            ),
            Err(CheckoutError::Cancelled)
        );
        let missing_guard = tempfile::tempdir().unwrap();
        let missing = SourceCheckout {
            path: missing_guard.path().join("never-created"),
            repository_url: "https://example.invalid/repo".into(),
            revision: "a".repeat(40),
        };
        assert_eq!(missing.cleanup(), Err(CheckoutError::CleanupFailed));
    }

    #[test]
    fn checkout_failure_codes_are_stable_and_unique() {
        let failures = [
            CheckoutError::InvalidRequest,
            CheckoutError::Offline,
            CheckoutError::Unavailable,
            CheckoutError::RepositoryIdentityMismatch,
            CheckoutError::RevisionMismatch,
            CheckoutError::ResourceLimit,
            CheckoutError::Cancelled,
            CheckoutError::CleanupFailed,
        ];
        let mut codes = std::collections::BTreeSet::new();
        for failure in failures {
            assert!(failure.code().starts_with("source_checkout_"));
            assert!(codes.insert(failure.code()));
        }
        assert_eq!(codes.len(), 8);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_staging_members_are_rejected() {
        let root = std::env::temp_dir().join(format!("asb-source-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink("/tmp", root.join("link")).unwrap();
        assert_eq!(
            enforce_bounds(&root, &CheckoutLimits::default()),
            Err(CheckoutError::ResourceLimit)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn unreadable_existing_staging_fails_closed_while_child_runs() {
        use std::os::unix::fs::PermissionsExt;
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let unreadable =
            std::env::temp_dir().join(format!("asb-unreadable-checkout-{}", std::process::id()));
        std::fs::create_dir_all(&unreadable).unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
        let mut command = Command::new("sleep");
        command.arg("1");
        let result = run_bounded_until(
            command,
            &request,
            Instant::now() + Duration::from_secs(2),
            Some(&unreadable),
        );
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&unreadable).unwrap();
        assert_eq!(result, Err(CheckoutError::Unavailable));
    }

    #[test]
    fn exact_fixture_checkout_is_returned_and_cleanup_is_explicit() {
        let _semaphore_guard = crate::test_fixture_semaphore::acquire();
        let _fixture_lock_guard = fixture_lock();
        let (source, revision, staging) = fixture_repository();
        let mut request = CheckoutRequest::for_tool(
            &tool(&format!("file://{}", source.display()), &revision),
            staging.clone(),
        )
        .unwrap();
        request.repository_url = format!("file://{}", source.display());
        let checkout = acquire(&request).unwrap();
        assert_eq!(checkout.revision, revision);
        assert!(checkout.path().join("README").is_file());
        checkout.cleanup().unwrap();
        let _ = std::fs::remove_dir_all(source.parent().unwrap());
    }

    #[test]
    fn unavailable_repository_is_transactionally_cleaned() {
        let _fixture_guard = crate::test_fixture_semaphore::acquire();
        let (source, revision, staging) = fixture_repository();
        let missing_repo = source.parent().unwrap().join("missing-repository");
        let request = CheckoutRequest::for_tool(
            &tool(&format!("file://{}", missing_repo.display()), &revision),
            staging.clone(),
        )
        .unwrap();
        assert_eq!(acquire(&request), Err(CheckoutError::Unavailable));
        assert!(staging.read_dir().unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(source.parent().unwrap());
    }

    #[test]
    fn bounded_capture_preserves_small_identity_observation_without_unbounded_output() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("identity");
        let mut command = Command::new("printf");
        command.arg("immutable-revision\n");
        run_bounded_capture(
            command,
            &output,
            &request,
            Instant::now() + Duration::from_secs(2),
            Some(root.path()),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            "immutable-revision\n"
        );
    }

    #[test]
    fn bounded_capture_rejects_oversized_identity_observation() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("identity");
        let mut command = Command::new("printf");
        command.arg("%s").arg("x".repeat(257));
        assert_eq!(
            run_bounded_capture(
                command,
                &output,
                &request,
                Instant::now() + Duration::from_secs(2),
                Some(root.path()),
            ),
            Err(CheckoutError::ResourceLimit)
        );
        assert_eq!(std::fs::metadata(output).unwrap().len(), 257);
    }

    #[test]
    fn aggregate_deadline_and_file_count_limits_fail_closed() {
        let request = CheckoutRequest::for_tool(
            &tool("https://example.invalid/repo", &"a".repeat(40)),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("identity");
        let mut command = Command::new("sleep");
        command.arg("1");
        assert_eq!(
            run_bounded_capture(
                command,
                &output,
                &request,
                Instant::now() + Duration::from_millis(10),
                Some(root.path()),
            ),
            Err(CheckoutError::Offline)
        );

        std::fs::write(root.path().join("one"), b"1").unwrap();
        std::fs::write(root.path().join("two"), b"2").unwrap();
        let limits = CheckoutLimits {
            max_files: 1,
            ..CheckoutLimits::default()
        };
        assert_eq!(
            enforce_bounds(root.path(), &limits),
            Err(CheckoutError::ResourceLimit)
        );
    }
}
