//! Deterministic C0 contract snapshot generation and verification.

use std::{
    collections::BTreeSet,
    error::Error,
    ffi::OsStr,
    fs,
    io,
    path::{Component, Path},
};

pub(crate) mod fixtures;
mod manifest;
mod model;
mod openapi;
mod realtime;
pub(crate) mod sha256;

pub type BoxError = Box<dyn Error + Send + Sync>;

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

pub fn generate(output: &Path, provenance_path: &Path) -> Result<usize, BoxError> {
    let provenance = manifest::load_provenance(provenance_path)?;
    let artifacts = expected_artifacts(&provenance)?;
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
    let expected = expected_artifacts(&provenance)?;
    validate_output_root(input)?;

    let expected_paths = expected
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<BTreeSet<_>>();
    let actual_paths = collect_relative_files(input)?;
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
    manifest::verify(&manifest_bytes, &provenance, &actual_non_manifest)?;
    Ok(expected.len())
}

#[cfg(test)]
pub(crate) fn expected_paths(provenance_path: &Path) -> Result<Vec<String>, BoxError> {
    let provenance = manifest::load_provenance(provenance_path)?;
    Ok(expected_artifacts(&provenance)?
        .into_iter()
        .map(|artifact| artifact.path)
        .collect())
}

fn expected_artifacts(
    provenance: &manifest::Provenance,
) -> Result<Vec<Artifact>, BoxError> {
    let mut artifacts = Vec::new();
    artifacts.push(Artifact::json("openapi.json", &openapi::document()?)?);
    for (path, document) in realtime::documents()? {
        artifacts.push(Artifact::json(path, &document)?);
    }
    for (path, document) in fixtures::documents() {
        artifacts.push(Artifact::json(path, &document)?);
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    validate_artifact_paths(&artifacts)?;

    let manifest = manifest::artifact(provenance, &artifacts)?;
    artifacts.push(manifest);
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    validate_artifact_paths(&artifacts)?;
    Ok(artifacts)
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
