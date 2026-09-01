//! Deterministic C0 contract snapshot generation and verification.

use std::{
    collections::BTreeSet,
    error::Error,
    ffi::OsStr,
    fs, io,
    path::{Component, Path, PathBuf},
};

pub(crate) mod fixtures;
mod manifest;
mod model;
mod openapi;
mod realtime;
mod selected;
pub(crate) mod sha256;

pub type BoxError = Box<dyn Error + Send + Sync>;

struct OwnerContribution {
    path: &'static str,
    bytes: &'static [u8],
}

macro_rules! contribution {
    ($path:literal) => {
        OwnerContribution {
            path: concat!("contributions/", $path),
            bytes: include_bytes!(concat!("../../contracts/contributions/", $path)),
        }
    };
}

// Owner contributions remain read-only inputs. They are intentionally not
// generated, enumerated by the snapshot manifest, or checksummed with it.
const OWNER_CONTRIBUTIONS: &[OwnerContribution] = &[
    contribution!("task-5/dto/operations.json"),
    contribution!("task-5/fixtures/mobile-auth-handoff.json"),
    contribution!("task-5/schemas/auth-wire.schema.json"),
    contribution!("task-6/dto/operations.json"),
    contribution!("task-6/fixtures/group-invite-flow.json"),
    contribution!("task-6/schemas/groups-wire.schema.json"),
    contribution!("task-6b/dto/operations.json"),
    contribution!("task-6b/fixtures/chatroom-history-read.json"),
    contribution!("task-6b/schemas/chatrooms-wire.schema.json"),
    contribution!("task-7/dto/operations.json"),
    contribution!("task-7/fixtures/topic-flow.json"),
    contribution!("task-7/schemas/topics-wire.schema.json"),
    contribution!("task-8/dto/operations.json"),
    contribution!("task-8/fixtures/media-flow.json"),
    contribution!("task-8/schemas/media-wire.schema.json"),
    contribution!("task-9/dto/operations.json"),
    contribution!("task-9/fixtures/notifications-push-flow.json"),
    contribution!("task-9/schemas/notifications-push-wire.schema.json"),
];

#[derive(Clone, Debug)]
pub(crate) struct Artifact {
    pub(crate) path: String,
    pub(crate) bytes: Vec<u8>,
}

impl Artifact {
    fn json(path: impl Into<String>, value: &impl serde::Serialize) -> Result<Self, BoxError> {
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        Ok(Self {
            path: path.into(),
            bytes,
        })
    }
}

#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "the production-composition test includes this module but exercises only the release-candidate seams"
    )
)]
pub fn generate(output: &Path, provenance_path: &Path) -> Result<usize, BoxError> {
    let provenance = manifest::load_provenance(provenance_path)?;
    let artifacts = expected_artifacts(&provenance, SnapshotProfile::C0)?;
    validate_output_root(output)?;

    let expected_paths = artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<BTreeSet<_>>();
    let existing_paths = collect_relative_files(output)?;
    let extra_paths = existing_paths
        .difference(&expected_paths)
        .cloned()
        .collect::<Vec<_>>();
    if !extra_paths.is_empty() {
        return Err(invalid_data(format!(
            "refusing to overwrite a contract tree with extra files: {extra_paths:?}"
        ))
        .into());
    }

    for artifact in &artifacts {
        write_artifact(output, artifact)?;
    }
    Ok(artifacts.len())
}

pub fn verify(input: &Path, provenance_path: &Path) -> Result<usize, BoxError> {
    let provenance = manifest::load_provenance(provenance_path)?;
    let profile = SnapshotProfile::from_manifest(&input.join("manifest.json"))?;
    let expected = expected_artifacts(&provenance, profile)?;
    validate_output_root(input)?;

    let expected_paths = expected
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<BTreeSet<_>>();
    let actual_paths = match profile {
        SnapshotProfile::C0 => collect_relative_files(input)?,
        SnapshotProfile::ReleaseCandidate => contract_artifact_paths(input)?,
    };
    if actual_paths != expected_paths {
        let missing = expected_paths
            .difference(&actual_paths)
            .cloned()
            .collect::<Vec<_>>();
        let extra = actual_paths
            .difference(&expected_paths)
            .cloned()
            .collect::<Vec<_>>();
        return Err(invalid_data(format!(
            "contract artifact set differs; missing={missing:?}, extra={extra:?}"
        ))
        .into());
    }

    let mut actual_non_manifest = Vec::new();
    let mut manifest_bytes = None;
    for expected_artifact in &expected {
        let actual_bytes = fs::read(input.join(&expected_artifact.path))?;
        if actual_bytes != expected_artifact.bytes {
            return Err(invalid_data(format!(
                "contract artifact differs: {}",
                expected_artifact.path
            ))
            .into());
        }
        if expected_artifact.path == "manifest.json" {
            manifest_bytes = Some(actual_bytes);
        } else {
            actual_non_manifest.push(Artifact {
                path: expected_artifact.path.clone(),
                bytes: actual_bytes,
            });
        }
    }
    let manifest_bytes = manifest_bytes
        .ok_or_else(|| invalid_data("contract manifest is absent after allowlist verification"))?;
    manifest::verify(&manifest_bytes, &provenance, &actual_non_manifest, profile)?;
    Ok(expected.len())
}

/// Materializes the approved release-candidate snapshot in the committed
/// workspace contract tree.
///
/// This deliberately has a fixed destination and provenance input so a
/// release-candidate invocation cannot redirect generated artifacts or infer
/// release metadata from the environment.
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "legacy C0 contract tests include this module but do not exercise release-candidate materialization"
    )
)]
pub fn generate_release_candidate(
    output: &Path,
    provenance_path: &Path,
) -> Result<usize, BoxError> {
    let (workspace_output, workspace_provenance) =
        validate_release_candidate_paths(output, provenance_path)?;
    require_owner_contributions(&workspace_output)?;
    let provenance = manifest::load_provenance(&workspace_provenance)?;
    generate_profile(
        &workspace_output,
        &provenance,
        SnapshotProfile::ReleaseCandidate,
    )
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "Task-12 production-composition RED coverage consumes this compile seam through the included generator module"
)]
pub(crate) fn generate_disposable(
    output: &Path,
    provenance_path: &Path,
) -> Result<usize, BoxError> {
    validate_disposable_pair(output, provenance_path)?;
    reject_disposable_contributions(output)?;
    let provenance = manifest::load_provenance(provenance_path)?;
    generate_profile(output, &provenance, SnapshotProfile::ReleaseCandidate)
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "contract integration tests consume this helper through the included generator module"
)]
pub(crate) fn expected_paths(provenance_path: &Path) -> Result<Vec<String>, BoxError> {
    let provenance = manifest::load_provenance(provenance_path)?;
    Ok(expected_artifacts(&provenance, SnapshotProfile::C0)?
        .into_iter()
        .map(|artifact| artifact.path)
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotProfile {
    C0,
    ReleaseCandidate,
}

impl SnapshotProfile {
    fn from_manifest(path: &Path) -> Result<Self, BoxError> {
        let bytes = fs::read(path)?;
        if manifest::is_release_candidate(&bytes)? {
            Ok(Self::ReleaseCandidate)
        } else {
            Ok(Self::C0)
        }
    }
}

fn generate_profile(
    output: &Path,
    provenance: &manifest::Provenance,
    profile: SnapshotProfile,
) -> Result<usize, BoxError> {
    let artifacts = expected_artifacts(provenance, profile)?;
    validate_output_root(output)?;
    let expected_paths = artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<BTreeSet<_>>();
    let existing_paths = contract_artifact_paths(output)?;
    let extra_paths = existing_paths
        .difference(&expected_paths)
        .cloned()
        .collect::<Vec<_>>();
    if !extra_paths.is_empty() {
        return Err(invalid_data(format!(
            "refusing to overwrite a contract tree with extra files: {extra_paths:?}"
        ))
        .into());
    }
    for artifact in &artifacts {
        write_artifact(output, artifact)?;
    }
    Ok(artifacts.len())
}

#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "legacy C0 contract tests include this module but do not exercise release-candidate materialization"
    )
)]
pub(crate) fn validate_release_candidate_paths(
    output: &Path,
    provenance_path: &Path,
) -> Result<(PathBuf, PathBuf), BoxError> {
    const OUTPUT: &str = "contracts";
    const PROVENANCE: &str = "tests/production_composition/fixtures/contract_generation/dirty.json";

    reject_parent_traversal(output, "release-candidate output")?;
    reject_parent_traversal(provenance_path, "release-candidate provenance")?;

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_output =
        validate_approved_workspace_path(workspace_root, output, Path::new(OUTPUT), "output")?;
    let workspace_provenance = validate_approved_workspace_path(
        workspace_root,
        provenance_path,
        Path::new(PROVENANCE),
        "provenance",
    )?;
    Ok((workspace_output, workspace_provenance))
}

#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "legacy C0 contract tests include this module but do not exercise release-candidate materialization"
    )
)]
fn reject_parent_traversal(path: &Path, label: &str) -> Result<(), BoxError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(invalid_data(format!(
            "{label} must not contain parent traversal: {}",
            path.display()
        ))
        .into());
    }
    Ok(())
}

#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "legacy C0 contract tests include this module but do not exercise release-candidate materialization"
    )
)]
fn validate_approved_workspace_path(
    workspace_root: &Path,
    input: &Path,
    relative: &Path,
    label: &str,
) -> Result<PathBuf, BoxError> {
    let absolute = workspace_root.join(relative);
    if input.as_os_str() != relative.as_os_str() && input.as_os_str() != absolute.as_os_str() {
        return Err(invalid_data(format!(
            "release-candidate {label} must be exactly {} or {}",
            relative.display(),
            absolute.display()
        ))
        .into());
    }
    validate_real_workspace_components(workspace_root, relative, label)?;
    Ok(absolute)
}

#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "legacy C0 contract tests include this module but do not exercise release-candidate materialization"
    )
)]
fn validate_real_workspace_components(
    workspace_root: &Path,
    relative: &Path,
    label: &str,
) -> Result<(), BoxError> {
    let mut current = workspace_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(invalid_data(format!(
                "approved release-candidate {label} contains a non-normal component"
            ))
            .into());
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_data(format!(
                "release-candidate {label} must not traverse a symlink: {}",
                current.display()
            ))
            .into());
        }
    }
    Ok(())
}

fn expected_artifacts(
    provenance: &manifest::Provenance,
    profile: SnapshotProfile,
) -> Result<Vec<Artifact>, BoxError> {
    let mut artifacts = Vec::new();
    let openapi = match profile {
        SnapshotProfile::C0 => openapi::document()?,
        SnapshotProfile::ReleaseCandidate => openapi::document_release_candidate()?,
    };
    artifacts.push(Artifact::json("openapi.json", &openapi)?);
    let realtime_documents = match profile {
        SnapshotProfile::C0 => realtime::documents()?,
        SnapshotProfile::ReleaseCandidate => realtime::documents_release_candidate()?,
    };
    for (path, document) in realtime_documents {
        artifacts.push(Artifact::json(path, &document)?);
    }
    let fixture_documents = match profile {
        SnapshotProfile::C0 => fixtures::documents(),
        SnapshotProfile::ReleaseCandidate => fixtures::documents_release_candidate(),
    };
    for (path, document) in fixture_documents {
        artifacts.push(Artifact::json(path, &document)?);
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    validate_artifact_paths(&artifacts)?;

    let manifest = manifest::artifact(provenance, &artifacts, profile)?;
    artifacts.push(manifest);
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    validate_artifact_paths(&artifacts)?;
    Ok(artifacts)
}

#[cfg(test)]
fn validate_disposable_pair(output: &Path, provenance_path: &Path) -> Result<(), BoxError> {
    const FIXTURES: &[&str] = &["dirty", "clean-prepublication", "future-transition"];
    if provenance_path.is_symlink()
        || output
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(invalid_data(
            "disposable inputs and outputs must not traverse symlinks or parent paths",
        )
        .into());
    }
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/production_composition/fixtures/contract_generation");
    let mut selected_fixture = None;
    for fixture in FIXTURES {
        let candidate = fixture_root.join(format!("{fixture}.json"));
        if provenance_path == candidate {
            validate_real_path_components(
                Path::new(env!("CARGO_MANIFEST_DIR")),
                Path::new("tests/production_composition/fixtures/contract_generation")
                    .join(format!("{fixture}.json"))
                    .as_path(),
            )?;
            selected_fixture = Some(*fixture);
            break;
        }
    }
    let fixture = selected_fixture.ok_or_else(|| {
        invalid_data(
            "disposable contract generation accepts only the three committed provenance fixtures",
        )
    })?;

    let temporary_root = std::env::temp_dir().join("jamye-task-12-contract-generation");
    match fs::symlink_metadata(&temporary_root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(
                invalid_data("Task-12 disposable temporary root must be a real directory").into(),
            );
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(
                invalid_data("Task-12 disposable temporary root must already exist").into(),
            );
        }
        Err(error) => return Err(error.into()),
    }
    let relative_output = output.strip_prefix(&temporary_root).map_err(|_| {
        invalid_data("disposable output must stay beneath the task-owned temporary root")
    })?;
    let mut inspected = temporary_root.clone();
    for component in relative_output.components() {
        let Component::Normal(component) = component else {
            return Err(
                invalid_data("disposable output contains a non-normal path component").into(),
            );
        };
        inspected.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&inspected)
            && metadata.file_type().is_symlink()
        {
            return Err(invalid_data("disposable output must not traverse a symlink").into());
        }
    }
    let canonical_root = fs::canonicalize(&temporary_root).unwrap_or(temporary_root.clone());
    let expected_root = canonical_root.join(fixture);
    let output_parent = output
        .parent()
        .ok_or_else(|| invalid_data("disposable output has no parent"))?;
    let canonical_parent =
        fs::canonicalize(output_parent).unwrap_or_else(|_| output_parent.to_path_buf());
    if canonical_parent != expected_root || output.file_name().is_none() {
        return Err(invalid_data(
            "disposable output must stay beneath its matching task-owned fixture root",
        )
        .into());
    }
    if output.exists() && fs::symlink_metadata(output)?.file_type().is_symlink() {
        return Err(invalid_data("disposable output must not be a symlink").into());
    }
    Ok(())
}

#[cfg(test)]
fn reject_disposable_contributions(output: &Path) -> Result<(), BoxError> {
    if !output.exists() {
        return Ok(());
    }
    let paths = collect_relative_files(output)?;
    if paths
        .iter()
        .any(|path| path.split('/').next() == Some("contributions"))
    {
        return Err(invalid_data(
            "disposable release-candidate generation rejects every pre-existing contribution path",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
fn validate_real_path_components(root: &Path, relative: &Path) -> Result<(), BoxError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(invalid_data(
                "approved disposable provenance contains a non-normal component",
            )
            .into());
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(
                invalid_data("approved disposable provenance must not traverse a symlink").into(),
            );
        }
    }
    Ok(())
}

fn validate_artifact_paths(artifacts: &[Artifact]) -> Result<(), BoxError> {
    let mut previous = None;
    for artifact in artifacts {
        let path = Path::new(&artifact.path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(invalid_data(format!(
                "generated artifact path is unsafe: {}",
                artifact.path
            ))
            .into());
        }
        if previous == Some(artifact.path.as_str()) {
            return Err(invalid_data(format!(
                "duplicate generated artifact path: {}",
                artifact.path
            ))
            .into());
        }
        previous = Some(artifact.path.as_str());
    }
    Ok(())
}

fn validate_output_root(root: &Path) -> Result<(), BoxError> {
    if !root.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_data(format!(
            "contract root must be a real directory: {}",
            root.display()
        ))
        .into());
    }
    Ok(())
}

fn collect_relative_files(root: &Path) -> Result<BTreeSet<String>, BoxError> {
    let mut files = BTreeSet::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_directory(root, root, &mut files)?;
    Ok(files)
}

fn contract_artifact_paths(root: &Path) -> Result<BTreeSet<String>, BoxError> {
    let mut paths = collect_relative_files(root)?;
    validate_owner_contributions(root, &paths)?;
    paths.retain(|path| !path.starts_with("contributions/"));
    Ok(paths)
}

fn validate_owner_contributions(root: &Path, paths: &BTreeSet<String>) -> Result<(), BoxError> {
    let actual = paths
        .iter()
        .filter(|path| path.starts_with("contributions/"))
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual.is_empty() {
        return Ok(());
    }
    let expected = OWNER_CONTRIBUTIONS
        .iter()
        .map(|contribution| contribution.path.to_owned())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(invalid_data(format!(
            "owner contributions must be absent or the complete exact allowlist; actual={actual:?}"
        ))
        .into());
    }
    for contribution in OWNER_CONTRIBUTIONS {
        let bytes = fs::read(root.join(contribution.path))?;
        if bytes != contribution.bytes {
            return Err(invalid_data(format!(
                "owner contribution bytes differ from the compile-time input: {}",
                contribution.path
            ))
            .into());
        }
    }
    Ok(())
}

#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "legacy C0 contract tests include this module but do not exercise release-candidate materialization"
    )
)]
fn require_owner_contributions(root: &Path) -> Result<(), BoxError> {
    let paths = collect_relative_files(root)?;
    let actual = paths
        .iter()
        .filter(|path| path.starts_with("contributions/"))
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected = OWNER_CONTRIBUTIONS
        .iter()
        .map(|contribution| contribution.path.to_owned())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(invalid_data(format!(
            "release-candidate materialization requires the complete exact owner contribution allowlist; actual={actual:?}"
        ))
        .into());
    }
    for contribution in OWNER_CONTRIBUTIONS {
        let bytes = fs::read(root.join(contribution.path))?;
        if bytes != contribution.bytes {
            return Err(invalid_data(format!(
                "release-candidate owner contribution bytes differ from the compile-time input: {}",
                contribution.path
            ))
            .into());
        }
    }
    Ok(())
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
) -> Result<(), BoxError> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_data(format!(
                "contract tree contains a symlink: {}",
                path.display()
            ))
            .into());
        }
        if metadata.is_dir() {
            collect_directory(root, &path, files)?;
        } else if metadata.is_file() {
            files.insert(relative_contract_path(root, &path)?);
        } else {
            return Err(invalid_data(format!(
                "contract tree contains a non-file entry: {}",
                path.display()
            ))
            .into());
        }
    }
    Ok(())
}

fn relative_contract_path(root: &Path, path: &Path) -> Result<String, BoxError> {
    let relative = path.strip_prefix(root)?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(os_string(value)?),
            _ => return Err(invalid_data("contract path contains a non-normal component").into()),
        }
    }
    Ok(parts.join("/"))
}

fn os_string(value: &OsStr) -> Result<String, BoxError> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid_data("contract path must be valid UTF-8").into())
}

fn write_artifact(root: &Path, artifact: &Artifact) -> Result<(), BoxError> {
    let destination = root.join(&artifact.path);
    let parent = destination
        .parent()
        .ok_or_else(|| invalid_data("generated artifact has no parent"))?;
    fs::create_dir_all(parent)?;

    let file_name = destination
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| invalid_data("generated artifact filename is invalid"))?;
    let temporary = destination.with_file_name(format!(".{file_name}.task-3b.tmp"));
    if temporary.exists() {
        return Err(invalid_data(format!(
            "stale contract temporary file exists: {}",
            temporary.display()
        ))
        .into());
    }
    fs::write(&temporary, &artifact.bytes)?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
