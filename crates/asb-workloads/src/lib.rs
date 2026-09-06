// SPDX-License-Identifier: MIT
//! Offline, bounded original engineering workloads and protected graders.

use asb_protocol::{Id, WorkloadManifest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

/// Maximum number of regular files accepted from one attempt workspace.
pub const MAX_WORKSPACE_FILES: usize = 32;
/// Maximum aggregate bytes accepted from one attempt workspace.
pub const MAX_WORKSPACE_BYTES: u64 = 1024 * 1024;
/// Maximum bytes accepted from one individual file.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
const OWNER_FILE: &str = ".asb-workload-owner";
const WORKSPACE_DIR: &str = "workspace";
const CONTENT_DOMAIN: &[u8] = b"asb-workload-content-v1";

/// Stable identifiers for the original v1 fixture catalogue.
pub const FIXTURE_IDS: [&str; 7] = [
    "original.bug-fix",
    "original.feature-addition",
    "original.refactoring",
    "original.test-generation",
    "original.dependency-migration",
    "original.build-repair",
    "original.repository-navigation",
];

/// A workload lifecycle failure that never includes submitted file contents.
#[derive(Debug)]
pub enum WorkloadError {
    /// The requested fixture is not in the pinned catalogue.
    UnknownFixture,
    /// A path or prepared-root ownership boundary was invalid.
    UnsafePath,
    /// A destination already exists or contains caller-owned material.
    DestinationExists,
    /// A manifest or its content pin was inconsistent.
    InvalidManifest,
    /// The submitted workspace exceeded a fixed count or byte bound.
    LimitExceeded,
    /// A submitted tree contained an unsupported object or unexpected file.
    InvalidWorkspace,
    /// A bounded filesystem operation failed.
    Io(io::Error),
}

impl fmt::Display for WorkloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFixture => formatter.write_str("unknown workload fixture"),
            Self::UnsafePath => formatter.write_str("unsafe workload path or ownership"),
            Self::DestinationExists => formatter.write_str("workload destination already exists"),
            Self::InvalidManifest => formatter.write_str("invalid pinned workload manifest"),
            Self::LimitExceeded => formatter.write_str("workload file or byte limit exceeded"),
            Self::InvalidWorkspace => formatter.write_str("invalid workload workspace"),
            Self::Io(error) => write!(formatter, "workload I/O failed: {error}"),
        }
    }
}

impl std::error::Error for WorkloadError {}

impl From<io::Error> for WorkloadError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Content-free result produced only by an independent protected grader.
///
/// Callers can inspect but cannot construct or mutate grading evidence. For
/// example, forging a passing report is rejected outside this crate:
///
/// ```compile_fail
/// use asb_workloads::GradeReport;
///
/// let _forged = GradeReport {
///     passed: true,
///     failed_checks: Vec::new(),
///     scoring_version: "forged".to_owned(),
/// };
/// ```
///
/// Existing reports cannot have their outcome reassigned either:
///
/// ```compile_fail
/// # fn forge(mut report: asb_workloads::GradeReport) {
/// report.passed = true;
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradeReport {
    passed: bool,
    failed_checks: Vec<&'static str>,
    scoring_version: String,
}

impl GradeReport {
    /// True only when every named check passed.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }

    /// Stable failed-check identifiers; submitted content is never copied.
    #[must_use]
    pub fn failed_checks(&self) -> &[&'static str] {
        &self.failed_checks
    }

    /// Independently versioned scorer identity from the workload manifest.
    #[must_use]
    pub fn scoring_version(&self) -> &str {
        &self.scoring_version
    }
}

/// A prepared attempt root with an agent-writable workspace child.
#[derive(Debug)]
pub struct PreparedWorkload {
    fixture: &'static Fixture,
    root: PathBuf,
}

impl PreparedWorkload {
    /// Directory exposed to the agent. Grader and reference data are not copied here.
    #[must_use]
    pub fn workspace(&self) -> PathBuf {
        self.root.join(WORKSPACE_DIR)
    }

    /// Public task prompt for this exact content-pinned fixture.
    #[must_use]
    pub fn prompt(&self) -> &'static str {
        self.fixture.prompt
    }

    /// Evaluate the bounded submitted tree with protected task-specific checks.
    pub fn evaluate(&self) -> Result<GradeReport, WorkloadError> {
        verify_owner(&self.root, self.fixture.id)?;
        let files = read_workspace(&self.workspace())?;
        let failed_checks = (self.fixture.grade)(&files);
        Ok(GradeReport {
            passed: failed_checks.is_empty(),
            failed_checks,
            scoring_version: self.fixture.manifest()?.scoring_version,
        })
    }

    /// Restore the exact initial project after verifying root ownership.
    pub fn reset(&self) -> Result<(), WorkloadError> {
        verify_owner(&self.root, self.fixture.id)?;
        let workspace = self.workspace();
        remove_workspace(&workspace)?;
        fs::DirBuilder::new().mode(0o700).create(&workspace)?;
        write_initial(self.fixture, &workspace)
    }

    /// Remove the exact prepared root after verifying its private owner marker.
    pub fn cleanup(&self) -> Result<(), WorkloadError> {
        verify_owner(&self.root, self.fixture.id)?;
        fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}

/// Built-in workload lifecycle compatible with extension API v1 method meanings.
#[derive(Clone, Copy, Debug, Default)]
pub struct OriginalWorkloads;

impl OriginalWorkloads {
    /// Return all stable fixture identities in deterministic order.
    #[must_use]
    pub const fn fixture_ids() -> &'static [&'static str] {
        &FIXTURE_IDS
    }

    /// Describe one pinned workload using the shared v1 manifest type.
    pub fn describe(id: &str) -> Result<WorkloadManifest, WorkloadError> {
        fixture(id)?.manifest()
    }

    /// Acquire is a local pin check; these original fixtures require no network.
    pub fn acquire(id: &str) -> Result<(), WorkloadError> {
        fixture(id)?.validate().map(|_| ())
    }

    /// Prepare an exact clean fixture beneath a new absolute private root.
    pub fn prepare(id: &str, root: impl Into<PathBuf>) -> Result<PreparedWorkload, WorkloadError> {
        let fixture = fixture(id)?;
        fixture.validate()?;
        let root = root.into();
        if validate_absolute(&root).is_err() || root.exists() {
            return Err(if root.exists() {
                WorkloadError::DestinationExists
            } else {
                WorkloadError::UnsafePath
            });
        }
        let parent = root.parent().ok_or(WorkloadError::UnsafePath)?;
        if !parent.exists()
            || fs::symlink_metadata(parent)?.file_type().is_symlink()
            || !fs::symlink_metadata(parent)?.is_dir()
            || fs::canonicalize(parent)? != parent
        {
            return Err(WorkloadError::UnsafePath);
        }
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let result = (|| {
            let workspace = root.join(WORKSPACE_DIR);
            fs::DirBuilder::new().mode(0o700).create(&workspace)?;
            write_owner(&root, fixture.id)?;
            write_initial(fixture, &workspace)
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
        Ok(PreparedWorkload { fixture, root })
    }
}

#[derive(Debug)]
struct Fixture {
    id: &'static str,
    manifest_json: &'static str,
    prompt: &'static str,
    initial: &'static [(&'static str, &'static str)],
    allowed: &'static [&'static str],
    grade: fn(&BTreeMap<String, String>) -> Vec<&'static str>,
    #[cfg(test)]
    reference_patch: &'static str,
    #[cfg(test)]
    counterexamples: &'static [&'static str],
}

impl Fixture {
    fn manifest(&self) -> Result<WorkloadManifest, WorkloadError> {
        let manifest: WorkloadManifest =
            serde_json::from_str(self.manifest_json).map_err(|_| WorkloadError::InvalidManifest)?;
        if manifest.workload_id != Id(self.id.to_owned())
            || manifest.version != "1.0.0"
            || manifest.license != "MIT"
            || manifest.source_revision != "asb-original-v1"
            || manifest.scoring_version != "asb-original-oracle-v1"
            || !manifest.allowed_network_destinations.is_empty()
            || manifest.architectures != BTreeSet::from(["aarch64".into(), "x86_64".into()])
            || manifest.operating_systems != BTreeSet::from(["linux".into()])
            || manifest.fixture_bytes != initial_bytes(self.initial)
            || manifest.content_sha256 != content_digest(self.prompt, self.initial)
            || manifest.timeout_ms == 0
            || manifest.timeout_ms > 60_000
            || manifest.cpu_count != 1
            || manifest.memory_bytes != 256 * 1024 * 1024
        {
            return Err(WorkloadError::InvalidManifest);
        }
        Ok(manifest)
    }

    fn validate(&self) -> Result<WorkloadManifest, WorkloadError> {
        if self.initial.is_empty()
            || self.initial.len() > MAX_WORKSPACE_FILES
            || self.allowed.len() > MAX_WORKSPACE_FILES
            || self.prompt.len() > MAX_FILE_BYTES as usize
        {
            return Err(WorkloadError::InvalidManifest);
        }
        let mut paths = BTreeSet::new();
        for (path, contents) in self.initial {
            validate_relative(Path::new(path))?;
            if !paths.insert(*path) || contents.len() as u64 > MAX_FILE_BYTES {
                return Err(WorkloadError::InvalidManifest);
            }
        }
        for path in self.allowed {
            validate_relative(Path::new(path))?;
        }
        self.manifest()
    }
}

fn fixture(id: &str) -> Result<&'static Fixture, WorkloadError> {
    FIXTURES
        .iter()
        .find(|fixture| fixture.id == id)
        .ok_or(WorkloadError::UnknownFixture)
}

fn validate_relative(path: &Path) -> Result<(), WorkloadError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(WorkloadError::UnsafePath);
    }
    Ok(())
}

fn validate_absolute(path: &Path) -> Result<(), WorkloadError> {
    if !path.is_absolute() {
        return Err(WorkloadError::UnsafePath);
    }
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::RootDir))
        || components.any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(WorkloadError::UnsafePath);
    }
    Ok(())
}

fn write_owner(root: &Path, id: &str) -> Result<(), WorkloadError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join(OWNER_FILE))?;
    file.write_all(id.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn verify_owner(root: &Path, id: &str) -> Result<(), WorkloadError> {
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(WorkloadError::UnsafePath);
    }
    let owner = read_bounded(&root.join(OWNER_FILE))?;
    if owner != id {
        return Err(WorkloadError::UnsafePath);
    }
    Ok(())
}

fn write_initial(fixture: &Fixture, workspace: &Path) -> Result<(), WorkloadError> {
    for (relative, contents) in fixture.initial {
        let path = workspace.join(relative);
        let parent = path.parent().ok_or(WorkloadError::UnsafePath)?;
        fs::create_dir_all(parent)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
    }
    Ok(())
}

fn remove_workspace(workspace: &Path) -> Result<(), WorkloadError> {
    if fs::symlink_metadata(workspace)?.file_type().is_symlink() {
        return Err(WorkloadError::UnsafePath);
    }
    fs::remove_dir_all(workspace)?;
    Ok(())
}

fn read_workspace(workspace: &Path) -> Result<BTreeMap<String, String>, WorkloadError> {
    if fs::symlink_metadata(workspace)?.file_type().is_symlink() {
        return Err(WorkloadError::InvalidWorkspace);
    }
    let mut output = BTreeMap::new();
    let mut pending = vec![(workspace.to_path_buf(), PathBuf::new())];
    let mut total = 0_u64;
    while let Some((directory, relative)) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let child_relative = relative.join(entry.file_name());
            validate_relative(&child_relative)?;
            if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
                return Err(WorkloadError::InvalidWorkspace);
            }
            if kind.is_dir() {
                pending.push((entry.path(), child_relative));
                continue;
            }
            if output.len() == MAX_WORKSPACE_FILES {
                return Err(WorkloadError::LimitExceeded);
            }
            let metadata = entry.metadata()?;
            if metadata.len() > MAX_FILE_BYTES {
                return Err(WorkloadError::LimitExceeded);
            }
            total = total
                .checked_add(metadata.len())
                .ok_or(WorkloadError::LimitExceeded)?;
            if total > MAX_WORKSPACE_BYTES {
                return Err(WorkloadError::LimitExceeded);
            }
            let text = read_bounded(&entry.path())?;
            let key = child_relative
                .to_str()
                .ok_or(WorkloadError::InvalidWorkspace)?
                .replace('\\', "/");
            output.insert(key, text);
        }
    }
    Ok(output)
}

fn read_bounded(path: &Path) -> Result<String, WorkloadError> {
    let file = OpenOptions::new().read(true).open(path)?;
    let metadata = file.metadata()?;
    let link_metadata = fs::symlink_metadata(path)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(WorkloadError::InvalidWorkspace);
    }
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(WorkloadError::LimitExceeded);
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| WorkloadError::LimitExceeded)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(WorkloadError::LimitExceeded);
    }
    String::from_utf8(bytes).map_err(|_| WorkloadError::InvalidWorkspace)
}

fn initial_bytes(initial: &[(&str, &str)]) -> u64 {
    initial
        .iter()
        .map(|(_, contents)| contents.len() as u64)
        .sum()
}

fn content_digest(prompt: &str, initial: &[(&str, &str)]) -> String {
    let mut items: Vec<(&str, &str)> = initial.to_vec();
    items.push(("PROMPT.md", prompt));
    items.sort_unstable_by_key(|(path, _)| *path);
    let mut digest = Sha256::new();
    digest.update(CONTENT_DOMAIN);
    for (path, contents) in items {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((contents.len() as u64).to_be_bytes());
        digest.update(contents.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn exact_inventory(files: &BTreeMap<String, String>, allowed: &[&str]) -> Vec<&'static str> {
    let actual: BTreeSet<&str> = files.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = allowed.iter().copied().collect();
    if actual == expected {
        Vec::new()
    } else {
        vec!["workspace.inventory"]
    }
}

fn exact_content(files: &BTreeMap<String, String>, path: &str, expected: &str) -> bool {
    files.get(path).is_some_and(|value| value == expected)
}

fn grade_bug_fix(files: &BTreeMap<String, String>) -> Vec<&'static str> {
    let mut failed = exact_inventory(files, &["parser.py", "tests.txt"]);
    if !exact_content(files, "parser.py", BUG_EXPECTED_SOURCE)
        || !exact_content(files, "tests.txt", BUG_INITIAL[1].1)
    {
        failed.push("parser.crlf_only");
    }
    failed
}

fn grade_feature(files: &BTreeMap<String, String>) -> Vec<&'static str> {
    let mut failed = exact_inventory(files, &["main.go", "compat.txt"]);
    if !exact_content(files, "main.go", FEATURE_EXPECTED_SOURCE)
        || !exact_content(files, "compat.txt", FEATURE_INITIAL[1].1)
    {
        failed.push("cli.json_and_text");
    }
    failed
}

fn grade_refactor(files: &BTreeMap<String, String>) -> Vec<&'static str> {
    let mut failed = exact_inventory(files, &["src/lib.rs", "src/normalize.rs", "behavior.txt"]);
    if !exact_content(files, "src/lib.rs", REFACTOR_EXPECTED_LIB)
        || !exact_content(files, "src/normalize.rs", REFACTOR_EXPECTED_MODULE)
        || !exact_content(files, "behavior.txt", REFACTOR_INITIAL[1].1)
    {
        failed.push("refactor.module_boundary");
    }
    failed
}

fn grade_test_generation(files: &BTreeMap<String, String>) -> Vec<&'static str> {
    let mut failed = exact_inventory(files, &["abs.c", "tests/cases.txt"]);
    if !exact_content(files, "abs.c", TEST_INITIAL[0].1) {
        failed.push("tests.implementation_unchanged");
    }
    let Some(cases) = files.get("tests/cases.txt") else {
        failed.push("tests.mutants");
        return failed;
    };
    let mut parsed = Vec::new();
    for line in cases.lines().filter(|line| !line.trim().is_empty()) {
        let Some((input, expected)) = line.split_once('|') else {
            failed.push("tests.format");
            return failed;
        };
        let Ok(input) = input.trim().parse::<i64>() else {
            failed.push("tests.format");
            return failed;
        };
        let Ok(expected) = expected.trim().parse::<u64>() else {
            failed.push("tests.format");
            return failed;
        };
        parsed.push((input, expected));
    }
    if parsed.len() > 32 || parsed.is_empty() {
        failed.push("tests.count");
        return failed;
    }
    let correct = |input: i64| input.unsigned_abs();
    let mutants = [
        |input: i64| input as u64,
        |input: i64| input.saturating_abs() as u64,
        |input: i64| if input == 0 { 1 } else { input.unsigned_abs() },
    ];
    if parsed
        .iter()
        .any(|&(input, expected)| correct(input) != expected)
        || mutants.iter().any(|mutant| {
            parsed
                .iter()
                .all(|&(input, expected)| mutant(input) == expected)
        })
    {
        failed.push("tests.mutants");
    }
    failed
}

fn grade_dependency(files: &BTreeMap<String, String>) -> Vec<&'static str> {
    let mut failed = exact_inventory(files, &["Cargo.toml", "src/lib.rs", "vendor/INDEX"]);
    if !exact_content(files, "Cargo.toml", DEP_EXPECTED_MANIFEST)
        || !exact_content(files, "src/lib.rs", DEP_EXPECTED_SOURCE)
        || !exact_content(files, "vendor/INDEX", DEP_INITIAL[2].1)
    {
        failed.push("dependency.pinned_api");
    }
    failed
}

fn grade_build(files: &BTreeMap<String, String>) -> Vec<&'static str> {
    let mut failed = exact_inventory(files, &["Makefile", "main.c", "util.c", "util.h"]);
    if !exact_content(files, "Makefile", BUILD_EXPECTED_MAKEFILE)
        || !exact_content(files, "main.c", BUILD_INITIAL[1].1)
        || !exact_content(files, "util.c", BUILD_INITIAL[2].1)
        || !exact_content(files, "util.h", BUILD_INITIAL[3].1)
    {
        failed.push("build.complete_inputs");
    }
    failed
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavigationAnswer {
    entrypoint: String,
    parser: String,
    symbol: String,
    call_chain: Vec<String>,
}

fn grade_navigation(files: &BTreeMap<String, String>) -> Vec<&'static str> {
    let mut failed = exact_inventory(
        files,
        &[
            "cmd/main.rs",
            "src/config.rs",
            "src/runner.rs",
            "answer.json",
        ],
    );
    let answer = files
        .get("answer.json")
        .and_then(|text| serde_json::from_str::<NavigationAnswer>(text).ok());
    if answer.is_none_or(|answer| {
        answer.entrypoint != "cmd/main.rs"
            || answer.parser != "src/config.rs"
            || answer.symbol != "parse_limit"
            || answer.call_chain != ["main", "load", "parse_limit", "Runner::new"]
    }) {
        failed.push("navigation.causal_path");
    } else if !exact_content(files, "cmd/main.rs", NAV_INITIAL[0].1)
        || !exact_content(files, "src/config.rs", NAV_INITIAL[1].1)
        || !exact_content(files, "src/runner.rs", NAV_INITIAL[2].1)
    {
        failed.push("navigation.sources_unchanged");
    }
    failed
}

const BUG_INITIAL: &[(&str, &str)] = &[
    (
        "parser.py",
        "def parse_line(line):\n    return line.strip()\n",
    ),
    (
        "tests.txt",
        "alpha => alpha\nalpha<CR> => alpha\nalpha<SPACE> => alpha<SPACE>\n",
    ),
];
const BUG_EXPECTED_SOURCE: &str = "def parse_line(line):\n    if line.endswith(\"\\r\"):\n        line = line[:-1]\n    return line\n";
const FEATURE_INITIAL: &[(&str, &str)] = &[
    (
        "main.go",
        "package main\n\nimport \"fmt\"\n\nfunc main() {\n    message := \"hello\"\n    fmt.Println(message)\n}\n",
    ),
    (
        "compat.txt",
        "no arguments prints hello followed by newline\n--json prints a JSON object with message=hello\n",
    ),
];
const FEATURE_EXPECTED_SOURCE: &str = "package main\n\nimport (\n\t\"encoding/json\"\n\t\"fmt\"\n\t\"os\"\n)\n\nfunc main() {\n\tmessage := \"hello\"\n\tif len(os.Args) == 2 && os.Args[1] == \"--json\" {\n\t\tjson.NewEncoder(os.Stdout).Encode(map[string]string{\"message\": message})\n\t\treturn\n\t}\n\tfmt.Println(message)\n}\n";
const REFACTOR_INITIAL: &[(&str, &str)] = &[
    (
        "src/lib.rs",
        "pub fn label(input: &str) -> String {\n    slug(input)\n}\n\nfn slug(input: &str) -> String {\n    input.trim().to_ascii_lowercase().replace(' ', \"-\")\n}\n",
    ),
    ("behavior.txt", " Hello World  => hello-world\n"),
];
const REFACTOR_EXPECTED_LIB: &str =
    "mod normalize;\n\npub fn label(input: &str) -> String {\n    normalize::slug(input)\n}\n";
const REFACTOR_EXPECTED_MODULE: &str = "pub(crate) fn slug(input: &str) -> String {\n    input.trim().to_ascii_lowercase().replace(' ', \"-\")\n}\n";
const TEST_INITIAL: &[(&str, &str)] = &[
    (
        "abs.c",
        "unsigned long magnitude(long value) { return value < 0 ? -value : value; }\n",
    ),
    ("tests/cases.txt", "1|1\n2|2\n"),
];
const DEP_INITIAL: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        "[package]\nname = \"offline-wrap\"\nversion = \"0.1.0\"\n\n[dependencies]\ntextwrap = \"=0.15.2\"\n",
    ),
    (
        "src/lib.rs",
        "pub fn format(value: &str) -> String { textwrap::fill(value, 20) }\n",
    ),
    (
        "vendor/INDEX",
        "textwrap 0.15.2 sha256:old-fixture\ntextwrap 0.16.1 sha256:new-fixture\n",
    ),
];
const DEP_EXPECTED_MANIFEST: &str = "[package]\nname = \"offline-wrap\"\nversion = \"0.1.0\"\n\n[dependencies]\ntextwrap = \"=0.16.1\"\n";
const DEP_EXPECTED_SOURCE: &str =
    "pub fn format(value: &str) -> String {\n    textwrap::wrap(value, 20).join(\"\\n\")\n}\n";
const BUILD_INITIAL: &[(&str, &str)] = &[
    (
        "Makefile",
        "main: main.c util.h\n\t$(CC) main.c missing.c -o main\n",
    ),
    (
        "main.c",
        "#include \"util.h\"\nint main(void) { return answer() == 42 ? 0 : 1; }\n",
    ),
    (
        "util.c",
        "#include \"util.h\"\nint answer(void) { return 42; }\n",
    ),
    ("util.h", "int answer(void);\n"),
];
const BUILD_EXPECTED_MAKEFILE: &str = "main: main.c util.c util.h\n\t$(CC) main.c util.c -o main\n";
const NAV_INITIAL: &[(&str, &str)] = &[
    (
        "cmd/main.rs",
        "fn main() { let config = config::load(); let _runner = Runner::new(config); }\n",
    ),
    (
        "src/config.rs",
        "pub fn load() -> u64 { parse_limit(\"8\") }\nfn parse_limit(value: &str) -> u64 { value.parse().unwrap_or(1) }\n",
    ),
    (
        "src/runner.rs",
        "pub struct Runner; impl Runner { pub fn new(_limit: u64) -> Self { Self } }\n",
    ),
    ("answer.json", "{}\n"),
];

const FIXTURES: [Fixture; 7] = [
    Fixture {
        id: FIXTURE_IDS[0],
        manifest_json: include_str!("../fixtures/v1/bug-fix/manifest.json"),
        prompt: include_str!("../fixtures/v1/bug-fix/prompt.md"),
        initial: BUG_INITIAL,
        allowed: &["parser.py", "tests.txt"],
        grade: grade_bug_fix,
        #[cfg(test)]
        reference_patch: include_str!("../fixtures/v1/bug-fix/reference.patch"),
        #[cfg(test)]
        counterexamples: &[
            include_str!("../fixtures/v1/bug-fix/counterexample.patch"),
            include_str!("../fixtures/v1/bug-fix/adversarial.patch"),
        ],
    },
    Fixture {
        id: FIXTURE_IDS[1],
        manifest_json: include_str!("../fixtures/v1/feature-addition/manifest.json"),
        prompt: include_str!("../fixtures/v1/feature-addition/prompt.md"),
        initial: FEATURE_INITIAL,
        allowed: &["main.go", "compat.txt"],
        grade: grade_feature,
        #[cfg(test)]
        reference_patch: include_str!("../fixtures/v1/feature-addition/reference.patch"),
        #[cfg(test)]
        counterexamples: &[
            include_str!("../fixtures/v1/feature-addition/counterexample.patch"),
            include_str!("../fixtures/v1/feature-addition/adversarial.patch"),
        ],
    },
    Fixture {
        id: FIXTURE_IDS[2],
        manifest_json: include_str!("../fixtures/v1/refactoring/manifest.json"),
        prompt: include_str!("../fixtures/v1/refactoring/prompt.md"),
        initial: REFACTOR_INITIAL,
        allowed: &["src/lib.rs", "src/normalize.rs", "behavior.txt"],
        grade: grade_refactor,
        #[cfg(test)]
        reference_patch: include_str!("../fixtures/v1/refactoring/reference.patch"),
        #[cfg(test)]
        counterexamples: &[
            include_str!("../fixtures/v1/refactoring/counterexample.patch"),
            include_str!("../fixtures/v1/refactoring/adversarial.patch"),
        ],
    },
    Fixture {
        id: FIXTURE_IDS[3],
        manifest_json: include_str!("../fixtures/v1/test-generation/manifest.json"),
        prompt: include_str!("../fixtures/v1/test-generation/prompt.md"),
        initial: TEST_INITIAL,
        allowed: &["abs.c", "tests/cases.txt"],
        grade: grade_test_generation,
        #[cfg(test)]
        reference_patch: include_str!("../fixtures/v1/test-generation/reference.patch"),
        #[cfg(test)]
        counterexamples: &[
            include_str!("../fixtures/v1/test-generation/counterexample.patch"),
            include_str!("../fixtures/v1/test-generation/adversarial.patch"),
        ],
    },
    Fixture {
        id: FIXTURE_IDS[4],
        manifest_json: include_str!("../fixtures/v1/dependency-migration/manifest.json"),
        prompt: include_str!("../fixtures/v1/dependency-migration/prompt.md"),
        initial: DEP_INITIAL,
        allowed: &["Cargo.toml", "src/lib.rs", "vendor/INDEX"],
        grade: grade_dependency,
        #[cfg(test)]
        reference_patch: include_str!("../fixtures/v1/dependency-migration/reference.patch"),
        #[cfg(test)]
        counterexamples: &[
            include_str!("../fixtures/v1/dependency-migration/counterexample.patch"),
            include_str!("../fixtures/v1/dependency-migration/adversarial.patch"),
        ],
    },
    Fixture {
        id: FIXTURE_IDS[5],
        manifest_json: include_str!("../fixtures/v1/build-repair/manifest.json"),
        prompt: include_str!("../fixtures/v1/build-repair/prompt.md"),
        initial: BUILD_INITIAL,
        allowed: &["Makefile", "main.c", "util.c", "util.h"],
        grade: grade_build,
        #[cfg(test)]
        reference_patch: include_str!("../fixtures/v1/build-repair/reference.patch"),
        #[cfg(test)]
        counterexamples: &[
            include_str!("../fixtures/v1/build-repair/counterexample.patch"),
            include_str!("../fixtures/v1/build-repair/adversarial.patch"),
        ],
    },
    Fixture {
        id: FIXTURE_IDS[6],
        manifest_json: include_str!("../fixtures/v1/repository-navigation/manifest.json"),
        prompt: include_str!("../fixtures/v1/repository-navigation/prompt.md"),
        initial: NAV_INITIAL,
        allowed: &[
            "cmd/main.rs",
            "src/config.rs",
            "src/runner.rs",
            "answer.json",
        ],
        grade: grade_navigation,
        #[cfg(test)]
        reference_patch: include_str!("../fixtures/v1/repository-navigation/reference.patch"),
        #[cfg(test)]
        counterexamples: &[
            include_str!("../fixtures/v1/repository-navigation/counterexample.patch"),
            include_str!("../fixtures/v1/repository-navigation/adversarial.patch"),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn resolve_scratch_base(
        asb_scratch: Option<PathBuf>,
        cargo_target: Option<PathBuf>,
        fallback: PathBuf,
    ) -> PathBuf {
        let configured = asb_scratch
            .map(|path| ("ASB_TEST_SCRATCH", path, false))
            .or_else(|| cargo_target.map(|path| ("CARGO_TARGET_DIR", path, true)));
        let Some((name, path, is_target)) = configured else {
            return fallback;
        };
        assert!(path.is_absolute(), "{name} must be an absolute path");
        if is_target {
            path.join("asb-test-scratch")
        } else {
            path
        }
    }

    fn scratch_base() -> PathBuf {
        let base = resolve_scratch_base(
            std::env::var_os("ASB_TEST_SCRATCH").map(PathBuf::from),
            std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from),
            std::env::temp_dir(),
        );
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn root(label: &str) -> PathBuf {
        scratch_base().join(format!(
            "asb-workload-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn apply(workspace: &Path, patch: &str) {
        let mut child = Command::new("git")
            .args([
                "apply",
                "--recount",
                "--unidiff-zero",
                "--whitespace=nowarn",
                "-",
            ])
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(patch.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn test_scratch_selection_is_absolute_prioritized_and_fail_closed() {
        let fallback = std::env::temp_dir();
        assert!(fallback.is_absolute());
        let asb = fallback.join("asb-selected");
        let target = fallback.join("cargo-selected");
        assert_eq!(
            resolve_scratch_base(Some(asb.clone()), Some(target.clone()), fallback.clone()),
            asb
        );
        assert_eq!(
            resolve_scratch_base(None, Some(target.clone()), fallback.clone()),
            target.join("asb-test-scratch")
        );
        assert_eq!(resolve_scratch_base(None, None, fallback.clone()), fallback);
        assert!(
            std::panic::catch_unwind(|| resolve_scratch_base(
                Some(PathBuf::from("relative")),
                None,
                PathBuf::from("unused")
            ))
            .is_err()
        );
    }

    #[test]
    fn every_manifest_is_pinned_offline_and_portable() {
        assert_eq!(OriginalWorkloads::fixture_ids(), &FIXTURE_IDS);
        for id in FIXTURE_IDS {
            OriginalWorkloads::acquire(id).unwrap();
            let manifest = OriginalWorkloads::describe(id).unwrap();
            assert!(manifest.allowed_network_destinations.is_empty());
            assert_eq!(manifest.architectures.len(), 2);
            assert_eq!(manifest.operating_systems, BTreeSet::from(["linux".into()]));
            assert_eq!(manifest.content_sha256.len(), 64);
        }
        assert!(matches!(
            OriginalWorkloads::describe("unknown"),
            Err(WorkloadError::UnknownFixture)
        ));
    }

    #[test]
    fn reference_patches_pass_and_counterexamples_fail() {
        for fixture in &FIXTURES {
            let pass_root = root("reference");
            let prepared = OriginalWorkloads::prepare(fixture.id, &pass_root).unwrap();
            assert!(!prepared.workspace().join("reference.patch").exists());
            apply(&prepared.workspace(), fixture.reference_patch);
            assert_eq!(
                prepared.evaluate().unwrap().failed_checks(),
                Vec::<&str>::new(),
                "{}",
                fixture.id
            );
            prepared.cleanup().unwrap();

            for counterexample in fixture.counterexamples {
                let fail_root = root("counterexample");
                let prepared = OriginalWorkloads::prepare(fixture.id, &fail_root).unwrap();
                apply(&prepared.workspace(), counterexample);
                assert!(!prepared.evaluate().unwrap().passed(), "{}", fixture.id);
                prepared.cleanup().unwrap();
            }
        }
    }

    #[test]
    fn reset_restores_exact_initial_tree_and_cleanup_is_owned() {
        let attempt_root = root("reset");
        let prepared = OriginalWorkloads::prepare(FIXTURE_IDS[0], &attempt_root).unwrap();
        fs::write(prepared.workspace().join("parser.py"), "changed").unwrap();
        prepared.reset().unwrap();
        let expected: BTreeMap<String, String> = BUG_INITIAL
            .iter()
            .map(|(path, contents)| ((*path).to_owned(), (*contents).to_owned()))
            .collect();
        assert_eq!(read_workspace(&prepared.workspace()).unwrap(), expected);
        fs::write(attempt_root.join(OWNER_FILE), "wrong-owner").unwrap();
        assert!(matches!(
            &prepared.cleanup(),
            Err(WorkloadError::UnsafePath)
        ));
        assert!(attempt_root.exists());
        fs::write(attempt_root.join(OWNER_FILE), FIXTURE_IDS[0]).unwrap();
        prepared.cleanup().unwrap();
        assert!(!attempt_root.exists());
    }

    #[test]
    fn preparation_and_evaluation_reject_unsafe_or_unbounded_trees() {
        let existing = root("existing");
        fs::create_dir(&existing).unwrap();
        assert!(matches!(
            OriginalWorkloads::prepare(FIXTURE_IDS[0], &existing),
            Err(WorkloadError::DestinationExists)
        ));
        fs::remove_dir(&existing).unwrap();
        assert!(matches!(
            OriginalWorkloads::prepare(FIXTURE_IDS[0], "relative"),
            Err(WorkloadError::UnsafePath)
        ));

        let symlink_root = root("symlink");
        let prepared = OriginalWorkloads::prepare(FIXTURE_IDS[0], &symlink_root).unwrap();
        std::os::unix::fs::symlink("parser.py", prepared.workspace().join("alias.py")).unwrap();
        assert!(matches!(
            prepared.evaluate(),
            Err(WorkloadError::InvalidWorkspace)
        ));
        fs::remove_file(prepared.workspace().join("alias.py")).unwrap();
        fs::write(
            prepared.workspace().join("large.txt"),
            vec![b'x'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(matches!(
            prepared.evaluate(),
            Err(WorkloadError::LimitExceeded)
        ));
        fs::remove_dir_all(symlink_root).unwrap();
    }

    #[test]
    fn count_aggregate_and_root_normalization_limits_fail_closed() {
        assert!(matches!(
            OriginalWorkloads::prepare(FIXTURE_IDS[0], "/tmp/asb-parent/../escape"),
            Err(WorkloadError::UnsafePath)
        ));
        let count_root = root("count");
        let prepared = OriginalWorkloads::prepare(FIXTURE_IDS[0], &count_root).unwrap();
        for index in 0..=MAX_WORKSPACE_FILES {
            fs::write(prepared.workspace().join(format!("extra-{index}")), "x").unwrap();
        }
        assert!(matches!(
            prepared.evaluate(),
            Err(WorkloadError::LimitExceeded)
        ));
        fs::remove_dir_all(count_root).unwrap();

        let aggregate_root = root("aggregate");
        let prepared = OriginalWorkloads::prepare(FIXTURE_IDS[0], &aggregate_root).unwrap();
        for index in 0..5 {
            fs::write(
                prepared.workspace().join(format!("large-{index}")),
                vec![b'x'; 220 * 1024],
            )
            .unwrap();
        }
        assert!(matches!(
            prepared.evaluate(),
            Err(WorkloadError::LimitExceeded)
        ));
        fs::remove_dir_all(aggregate_root).unwrap();
    }

    #[test]
    fn unexpected_files_and_non_utf8_fail_without_content_disclosure() {
        let attempt_root = root("invalid");
        let prepared = OriginalWorkloads::prepare(FIXTURE_IDS[0], &attempt_root).unwrap();
        fs::write(
            prepared.workspace().join("answer.txt"),
            "self reported pass",
        )
        .unwrap();
        let report = prepared.evaluate().unwrap();
        assert_eq!(
            report.failed_checks(),
            vec!["workspace.inventory", "parser.crlf_only"]
        );
        fs::write(prepared.workspace().join("answer.txt"), [0xff]).unwrap();
        assert!(matches!(
            prepared.evaluate(),
            Err(WorkloadError::InvalidWorkspace)
        ));
        prepared.cleanup().unwrap();
    }

    fn inert_grade(_: &BTreeMap<String, String>) -> Vec<&'static str> {
        Vec::new()
    }

    fn test_fixture(
        manifest_json: &'static str,
        prompt: &'static str,
        initial: &'static [(&'static str, &'static str)],
        allowed: &'static [&'static str],
    ) -> Fixture {
        Fixture {
            id: FIXTURE_IDS[0],
            manifest_json,
            prompt,
            initial,
            allowed,
            grade: inert_grade,
            reference_patch: "",
            counterexamples: &[],
        }
    }

    #[test]
    fn error_reporting_and_manifest_validation_branches_are_covered() {
        let errors = [
            (WorkloadError::UnknownFixture, "unknown workload fixture"),
            (
                WorkloadError::UnsafePath,
                "unsafe workload path or ownership",
            ),
            (
                WorkloadError::DestinationExists,
                "workload destination already exists",
            ),
            (
                WorkloadError::InvalidManifest,
                "invalid pinned workload manifest",
            ),
            (
                WorkloadError::LimitExceeded,
                "workload file or byte limit exceeded",
            ),
            (
                WorkloadError::InvalidWorkspace,
                "invalid workload workspace",
            ),
        ];
        for (error, expected) in errors {
            assert_eq!(error.to_string(), expected);
        }
        let io_error = WorkloadError::from(io::Error::other("bounded failure"));
        assert_eq!(io_error.to_string(), "workload I/O failed: bounded failure");

        assert!(matches!(
            test_fixture("not-json", "", BUG_INITIAL, &["parser.py"]).manifest(),
            Err(WorkloadError::InvalidManifest)
        ));
        let baseline: serde_json::Value = serde_json::from_str(FIXTURES[0].manifest_json).unwrap();
        let mutations = [
            ("workload_id", serde_json::json!("wrong")),
            ("version", serde_json::json!("2.0.0")),
            ("license", serde_json::json!("unknown")),
            ("source_revision", serde_json::json!("moving")),
            ("scoring_version", serde_json::json!("different")),
            (
                "allowed_network_destinations",
                serde_json::json!(["example.test"]),
            ),
            ("architectures", serde_json::json!(["x86_64"])),
            ("operating_systems", serde_json::json!(["other"])),
            ("fixture_bytes", serde_json::json!(0)),
            ("content_sha256", serde_json::json!("00")),
            ("timeout_ms", serde_json::json!(0)),
            ("timeout_ms", serde_json::json!(60_001)),
            ("cpu_count", serde_json::json!(2)),
            ("memory_bytes", serde_json::json!(1)),
        ];
        for (field, value) in mutations {
            let mut document = baseline.clone();
            document[field] = value;
            let encoded: &'static str =
                Box::leak(serde_json::to_string(&document).unwrap().into_boxed_str());
            assert!(
                matches!(
                    test_fixture(
                        encoded,
                        FIXTURES[0].prompt,
                        BUG_INITIAL,
                        FIXTURES[0].allowed
                    )
                    .manifest(),
                    Err(WorkloadError::InvalidManifest)
                ),
                "{field}"
            );
        }

        static EMPTY: &[(&str, &str)] = &[];
        static DUPLICATE: &[(&str, &str)] = &[("same", "a"), ("same", "b")];
        static UNSAFE_INITIAL: &[(&str, &str)] = &[("../escape", "x")];
        static UNSAFE_ALLOWED: &[&str] = &["/escape"];
        for fixture in [
            test_fixture(FIXTURES[0].manifest_json, FIXTURES[0].prompt, EMPTY, &[]),
            test_fixture(
                FIXTURES[0].manifest_json,
                FIXTURES[0].prompt,
                DUPLICATE,
                &["same"],
            ),
            test_fixture(
                FIXTURES[0].manifest_json,
                FIXTURES[0].prompt,
                UNSAFE_INITIAL,
                &["escape"],
            ),
            test_fixture(
                FIXTURES[0].manifest_json,
                FIXTURES[0].prompt,
                BUG_INITIAL,
                UNSAFE_ALLOWED,
            ),
        ] {
            assert!(fixture.validate().is_err());
        }
        static TOO_MANY_INITIAL: [(&str, &str); MAX_WORKSPACE_FILES + 1] =
            [("same", "x"); MAX_WORKSPACE_FILES + 1];
        static TOO_MANY_ALLOWED: [&str; MAX_WORKSPACE_FILES + 1] =
            ["same"; MAX_WORKSPACE_FILES + 1];
        assert!(
            test_fixture(
                FIXTURES[0].manifest_json,
                FIXTURES[0].prompt,
                &TOO_MANY_INITIAL,
                &[]
            )
            .validate()
            .is_err()
        );
        assert!(
            test_fixture(
                FIXTURES[0].manifest_json,
                FIXTURES[0].prompt,
                BUG_INITIAL,
                &TOO_MANY_ALLOWED
            )
            .validate()
            .is_err()
        );
        let huge_prompt: &'static str =
            Box::leak("x".repeat(MAX_FILE_BYTES as usize + 1).into_boxed_str());
        assert!(
            test_fixture(FIXTURES[0].manifest_json, huge_prompt, BUG_INITIAL, &[])
                .validate()
                .is_err()
        );
        let huge_contents: &'static str =
            Box::leak("x".repeat(MAX_FILE_BYTES as usize + 1).into_boxed_str());
        let huge_initial: &'static [(&'static str, &'static str)] =
            Box::leak(vec![("huge", huge_contents)].into_boxed_slice());
        assert!(
            test_fixture(FIXTURES[0].manifest_json, "prompt", huge_initial, &["huge"])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn filesystem_object_boundaries_fail_closed() {
        assert!(inert_grade(&BTreeMap::new()).is_empty());
        let absent_parent = root("absent-parent").join("child");
        assert!(matches!(
            OriginalWorkloads::prepare(FIXTURE_IDS[0], absent_parent),
            Err(WorkloadError::UnsafePath)
        ));
        let actual_parent = root("actual-parent");
        fs::create_dir(&actual_parent).unwrap();
        let linked_parent = root("linked-parent");
        std::os::unix::fs::symlink(&actual_parent, &linked_parent).unwrap();
        assert!(matches!(
            OriginalWorkloads::prepare(FIXTURE_IDS[0], linked_parent.join("attempt")),
            Err(WorkloadError::UnsafePath)
        ));
        fs::remove_file(linked_parent).unwrap();
        fs::remove_dir(actual_parent).unwrap();
        let attempt_root = root("object-boundaries");
        let prepared = OriginalWorkloads::prepare(FIXTURE_IDS[0], &attempt_root).unwrap();
        let root_alias = root("root-alias");
        std::os::unix::fs::symlink(&attempt_root, &root_alias).unwrap();
        assert!(matches!(
            verify_owner(&root_alias, FIXTURE_IDS[0]),
            Err(WorkloadError::UnsafePath)
        ));
        fs::remove_file(root_alias).unwrap();
        let workspace = prepared.workspace();
        let moved = attempt_root.join("workspace-real");
        fs::rename(&workspace, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &workspace).unwrap();
        assert!(matches!(
            prepared.evaluate(),
            Err(WorkloadError::InvalidWorkspace)
        ));
        assert!(matches!(prepared.reset(), Err(WorkloadError::UnsafePath)));
        fs::remove_file(&workspace).unwrap();
        fs::rename(&moved, &workspace).unwrap();
        let alias = attempt_root.join("owner-alias");
        std::os::unix::fs::symlink(attempt_root.join(OWNER_FILE), &alias).unwrap();
        assert!(matches!(
            read_bounded(&alias),
            Err(WorkloadError::InvalidWorkspace)
        ));
        fs::remove_file(alias).unwrap();
        let oversized = attempt_root.join("oversized");
        fs::write(&oversized, vec![0_u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            read_bounded(&oversized),
            Err(WorkloadError::LimitExceeded)
        ));
        fs::remove_file(oversized).unwrap();
        prepared.cleanup().unwrap();
    }

    #[test]
    fn malformed_test_generation_vectors_are_rejected_without_execution() {
        let grade = |value: Option<&str>| {
            let mut files = BTreeMap::from([("abs.c".to_owned(), "source".to_owned())]);
            if let Some(value) = value {
                files.insert("tests/cases.txt".to_owned(), value.to_owned());
            }
            grade_test_generation(&files)
        };
        for value in [
            None,
            Some("missing separator"),
            Some("x|1"),
            Some("1|x"),
            Some(""),
        ] {
            assert!(!grade(value).is_empty());
        }
        let too_many = (0..33)
            .map(|value| format!("{value}|{value}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(grade(Some(&too_many)).contains(&"tests.count"));
        assert!(grade(Some("-1|2\n0|0\n1|1")).contains(&"tests.mutants"));
    }
}
