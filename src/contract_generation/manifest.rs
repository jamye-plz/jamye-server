//! Explicit provenance and deterministic snapshot checksum.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Artifact, BoxError, SnapshotProfile, invalid_data, openapi, realtime, sha256};

pub const CHECKSUM_ALGORITHM: &str = "sha256 over lexicographic path,NUL,decimal-length,NUL,bytes entries; manifest.json uses recursively key-sorted compact JSON without sha256; v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub server_tag: Option<String>,
    pub server_commit: String,
    pub contract_version: String,
    pub server_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestCore {
    server_tag: Option<String>,
    server_commit: String,
    contract_version: String,
    server_version: String,
    checksum_algorithm: String,
    artifacts: Vec<String>,
    operation_ids: Vec<String>,
    realtime_discriminants: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    server_tag: Option<String>,
    server_commit: String,
    contract_version: String,
    server_version: String,
    checksum_algorithm: String,
    artifacts: Vec<String>,
    operation_ids: Vec<String>,
    realtime_discriminants: Vec<String>,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseCandidateManifestCore {
    stage: String,
    server_tag: Option<String>,
    server_commit: String,
    contract_version: String,
    server_version: String,
    checksum_algorithm: String,
    artifacts: Vec<String>,
    operation_ids: Vec<String>,
    realtime_discriminants: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseCandidateManifest {
    stage: String,
    server_tag: Option<String>,
    server_commit: String,
    contract_version: String,
    server_version: String,
    checksum_algorithm: String,
    artifacts: Vec<String>,
    operation_ids: Vec<String>,
    realtime_discriminants: Vec<String>,
    sha256: String,
}

impl Manifest {
    fn core(&self) -> ManifestCore {
        ManifestCore {
            server_tag: self.server_tag.clone(),
            server_commit: self.server_commit.clone(),
            contract_version: self.contract_version.clone(),
            server_version: self.server_version.clone(),
            checksum_algorithm: self.checksum_algorithm.clone(),
            artifacts: self.artifacts.clone(),
            operation_ids: self.operation_ids.clone(),
            realtime_discriminants: self.realtime_discriminants.clone(),
        }
    }
}

pub fn load_provenance(path: &Path) -> Result<Provenance, BoxError> {
    let bytes = fs::read(path)?;
    let provenance: Provenance = serde_json::from_slice(&bytes)?;
    validate_provenance(&provenance)?;
    Ok(provenance)
}

pub fn artifact(
    provenance: &Provenance,
    non_manifest: &[Artifact],
    profile: SnapshotProfile,
) -> Result<Artifact, BoxError> {
    let mut artifact_paths = non_manifest
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>();
    artifact_paths.push("manifest.json".to_owned());
    artifact_paths.sort();

    let core = ManifestCore {
        server_tag: provenance.server_tag.clone(),
        server_commit: provenance.server_commit.clone(),
        contract_version: provenance.contract_version.clone(),
        server_version: provenance.server_version.clone(),
        checksum_algorithm: CHECKSUM_ALGORITHM.to_owned(),
        artifacts: artifact_paths,
        operation_ids: (if profile == SnapshotProfile::C0 {
            openapi::C0_OPERATION_IDS
        } else {
            openapi::OPERATION_IDS
        })
        .iter()
        .map(|value| (*value).to_owned())
        .collect(),
        realtime_discriminants: (if profile == SnapshotProfile::C0 {
            realtime::C0_REALTIME_DISCRIMINANTS
        } else {
            realtime::REALTIME_DISCRIMINANTS
        })
        .iter()
        .map(|value| (*value).to_owned())
        .collect(),
    };
    if profile == SnapshotProfile::C0 {
        let checksum = checksum(&core, non_manifest)?;
        let manifest = Manifest {
            server_tag: core.server_tag.clone(),
            server_commit: core.server_commit.clone(),
            contract_version: core.contract_version.clone(),
            server_version: core.server_version.clone(),
            checksum_algorithm: core.checksum_algorithm.clone(),
            artifacts: core.artifacts.clone(),
            operation_ids: core.operation_ids.clone(),
            realtime_discriminants: core.realtime_discriminants.clone(),
            sha256: checksum,
        };
        return Artifact::json("manifest.json", &manifest);
    }

    let release_core = ReleaseCandidateManifestCore {
        stage: "release_candidate".to_owned(),
        server_tag: core.server_tag.clone(),
        server_commit: core.server_commit.clone(),
        contract_version: core.contract_version.clone(),
        server_version: core.server_version.clone(),
        checksum_algorithm: core.checksum_algorithm.clone(),
        artifacts: core.artifacts.clone(),
        operation_ids: core.operation_ids.clone(),
        realtime_discriminants: core.realtime_discriminants.clone(),
    };
    let checksum = checksum(&release_core, non_manifest)?;
    let manifest = ReleaseCandidateManifest {
        stage: release_core.stage.clone(),
        server_tag: release_core.server_tag.clone(),
        server_commit: release_core.server_commit.clone(),
        contract_version: release_core.contract_version.clone(),
        server_version: release_core.server_version.clone(),
        checksum_algorithm: release_core.checksum_algorithm.clone(),
        artifacts: release_core.artifacts.clone(),
        operation_ids: release_core.operation_ids.clone(),
        realtime_discriminants: release_core.realtime_discriminants.clone(),
        sha256: checksum,
    };
    Artifact::json("manifest.json", &manifest)
}

pub fn verify(
    manifest_bytes: &[u8],
    provenance: &Provenance,
    non_manifest: &[Artifact],
    profile: SnapshotProfile,
) -> Result<(), BoxError> {
    if profile == SnapshotProfile::ReleaseCandidate {
        return verify_release_candidate(manifest_bytes, provenance, non_manifest);
    }
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)?;
    let actual_provenance = Provenance {
        server_tag: manifest.server_tag.clone(),
        server_commit: manifest.server_commit.clone(),
        contract_version: manifest.contract_version.clone(),
        server_version: manifest.server_version.clone(),
    };
    if &actual_provenance != provenance {
        return Err(invalid_data("manifest provenance differs from the explicit input").into());
    }
    if manifest.checksum_algorithm != CHECKSUM_ALGORITHM {
        return Err(invalid_data("manifest checksum algorithm differs").into());
    }

    let mut expected_paths = non_manifest
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>();
    expected_paths.push("manifest.json".to_owned());
    expected_paths.sort();
    if manifest.artifacts != expected_paths {
        return Err(invalid_data("manifest artifact allowlist differs").into());
    }
    let expected_operations = openapi::C0_OPERATION_IDS
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    if manifest.operation_ids != expected_operations {
        return Err(invalid_data("manifest operation IDs differ").into());
    }
    let expected_discriminants = realtime::C0_REALTIME_DISCRIMINANTS
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    if manifest.realtime_discriminants != expected_discriminants {
        return Err(invalid_data("manifest realtime discriminants differ").into());
    }

    let expected_checksum = checksum(&manifest.core(), non_manifest)?;
    if manifest.sha256 != expected_checksum {
        return Err(invalid_data(format!(
            "manifest checksum differs: expected {expected_checksum}, got {}",
            manifest.sha256
        ))
        .into());
    }
    Ok(())
}

pub fn is_release_candidate(manifest_bytes: &[u8]) -> Result<bool, BoxError> {
    let value: Value = serde_json::from_slice(manifest_bytes)?;
    Ok(value.get("stage").and_then(Value::as_str) == Some("release_candidate"))
}

fn verify_release_candidate(
    manifest_bytes: &[u8],
    provenance: &Provenance,
    non_manifest: &[Artifact],
) -> Result<(), BoxError> {
    let manifest: ReleaseCandidateManifest = serde_json::from_slice(manifest_bytes)?;
    if manifest.stage != "release_candidate" {
        return Err(invalid_data("release manifest stage differs").into());
    }
    let actual_provenance = Provenance {
        server_tag: manifest.server_tag.clone(),
        server_commit: manifest.server_commit.clone(),
        contract_version: manifest.contract_version.clone(),
        server_version: manifest.server_version.clone(),
    };
    if &actual_provenance != provenance {
        return Err(invalid_data("manifest provenance differs from the explicit input").into());
    }
    if manifest.checksum_algorithm != CHECKSUM_ALGORITHM {
        return Err(invalid_data("manifest checksum algorithm differs").into());
    }
    let mut expected_paths = non_manifest
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>();
    expected_paths.push("manifest.json".to_owned());
    expected_paths.sort();
    if manifest.artifacts != expected_paths {
        return Err(invalid_data("manifest artifact allowlist differs").into());
    }
    let expected_operations = openapi::OPERATION_IDS
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    if manifest.operation_ids != expected_operations {
        return Err(invalid_data("manifest operation IDs differ").into());
    }
    let expected_discriminants = realtime::REALTIME_DISCRIMINANTS
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    if manifest.realtime_discriminants != expected_discriminants {
        return Err(invalid_data("manifest realtime discriminants differ").into());
    }
    let core = ReleaseCandidateManifestCore {
        stage: manifest.stage.clone(),
        server_tag: manifest.server_tag.clone(),
        server_commit: manifest.server_commit.clone(),
        contract_version: manifest.contract_version.clone(),
        server_version: manifest.server_version.clone(),
        checksum_algorithm: manifest.checksum_algorithm.clone(),
        artifacts: manifest.artifacts.clone(),
        operation_ids: manifest.operation_ids.clone(),
        realtime_discriminants: manifest.realtime_discriminants.clone(),
    };
    let expected_checksum = checksum(&core, non_manifest)?;
    if manifest.sha256 != expected_checksum {
        return Err(invalid_data(format!(
            "manifest checksum differs: expected {expected_checksum}, got {}",
            manifest.sha256
        ))
        .into());
    }
    Ok(())
}

fn validate_provenance(provenance: &Provenance) -> Result<(), BoxError> {
    if provenance.server_version.is_empty() || provenance.contract_version.is_empty() {
        return Err(invalid_data("server_version and contract_version must be non-empty").into());
    }
    if provenance.server_commit == "dirty" {
        if provenance.server_tag.is_some() {
            return Err(invalid_data("dirty provenance must use server_tag=null").into());
        }
        return Ok(());
    }

    let commit_is_hex = matches!(provenance.server_commit.len(), 40 | 64)
        && provenance
            .server_commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    if !commit_is_hex {
        return Err(
            invalid_data("published server_commit must be a 40- or 64-digit hex ID").into(),
        );
    }
    if provenance.server_tag.as_deref().is_none_or(str::is_empty) {
        return Err(
            invalid_data("published provenance must include a non-empty server_tag").into(),
        );
    }
    Ok(())
}

fn checksum<T: Serialize>(core: &T, non_manifest: &[Artifact]) -> Result<String, BoxError> {
    let mut entries = non_manifest
        .iter()
        .map(|artifact| (artifact.path.clone(), artifact.bytes.clone()))
        .collect::<Vec<_>>();
    let core_value = serde_json::to_value(core)?;
    entries.push(("manifest.json".to_owned(), canonical_json(&core_value)?));
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut checksum_input = Vec::new();
    for (path, bytes) in entries {
        checksum_input.extend_from_slice(path.as_bytes());
        checksum_input.push(0);
        checksum_input.extend_from_slice(bytes.len().to_string().as_bytes());
        checksum_input.push(0);
        checksum_input.extend_from_slice(&bytes);
    }
    Ok(sha256::digest_hex(&checksum_input))
}

fn canonical_json(value: &Value) -> Result<Vec<u8>, BoxError> {
    fn append(value: &Value, output: &mut Vec<u8>) -> Result<(), serde_json::Error> {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                serde_json::to_writer(output, value)?;
            }
            Value::Array(items) => {
                output.push(b'[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    append(item, output)?;
                }
                output.push(b']');
            }
            Value::Object(object) => {
                output.push(b'{');
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    serde_json::to_writer(&mut *output, key)?;
                    output.push(b':');
                    let child = object.get(key).ok_or_else(|| {
                        <serde_json::Error as serde::de::Error>::custom(
                            "canonical JSON object key disappeared",
                        )
                    })?;
                    append(child, output)?;
                }
                output.push(b'}');
            }
        }
        Ok(())
    }

    let mut output = Vec::new();
    append(value, &mut output)?;
    Ok(output)
}
