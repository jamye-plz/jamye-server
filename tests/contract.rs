use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::Value;

#[path = "../src/contract_generation/mod.rs"]
mod contract_generation;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

#[test]
fn local_sha256_matches_standard_vectors() {
    assert_eq!(
        contract_generation::sha256::digest_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        contract_generation::sha256::digest_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn generation_is_byte_deterministic_and_c0_is_minimal() -> Result<(), Box<dyn Error + Send + Sync>> {
    let first = TemporaryDirectory::new("first")?;
    let second = TemporaryDirectory::new("second")?;
    let provenance = provenance_path();

    contract_generation::generate(first.path(), &provenance)?;
    contract_generation::generate(second.path(), &provenance)?;
    contract_generation::verify(first.path(), &provenance)?;
    contract_generation::verify(second.path(), &provenance)?;

    let paths = contract_generation::expected_paths(&provenance)?;
    for relative in &paths {
        assert_eq!(
            fs::read(first.path().join(relative))?,
            fs::read(second.path().join(relative))?,
            "artifact differs across generations: {relative}"
        );
    }

    let openapi: Value = serde_json::from_slice(&fs::read(first.path().join("openapi.json"))?)?;
    assert_eq!(openapi.get("openapi").and_then(Value::as_str), Some("3.1.0"));
    assert_eq!(operation_ids(&openapi)?, BTreeSet::from(["C4", "H1", "H2", "R1", "S1"]));

    let realtime_schema = fs::read_to_string(
        first
            .path()
            .join("realtime/message.created.schema.json"),
    )?;
    assert!(realtime_schema.contains("message.created"));
    assert!(!realtime_schema.contains("topic.created"));
    assert!(!realtime_schema.contains("transcript"));

    let handoff: Value = serde_json::from_slice(&fs::read(
        first.path().join("fixtures/mobile-sync-handoff.json"),
    )?)?;
    assert_eq!(handoff.get("execution_owner").and_then(Value::as_str), Some("jamye-app"));
    assert_eq!(
        handoff.get("atomicity_contract").and_then(Value::as_str),
        Some(contract_generation::fixtures::SQLITE_ATOMICITY_SENTENCE)
    );
    let observer = handoff
        .get("server_observer")
        .and_then(Value::as_object)
        .ok_or_else(|| std::io::Error::other("server_observer is missing"))?;
    assert_eq!(
        observer.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["last_cursor", "seen_event_ids"])
    );
    Ok(())
}

#[test]
fn verification_rejects_any_byte_drift() -> Result<(), Box<dyn Error + Send + Sync>> {
    let generated = TemporaryDirectory::new("drift")?;
    let provenance = provenance_path();
    contract_generation::generate(generated.path(), &provenance)?;

    let fixture_path = generated.path().join("fixtures/c4-normal.json");
    let mut bytes = fs::read(&fixture_path)?;
    bytes.extend_from_slice(b" ");
    fs::write(&fixture_path, bytes)?;

    assert!(contract_generation::verify(generated.path(), &provenance).is_err());
    Ok(())
}

#[test]
fn future_publication_provenance_is_explicit_and_deterministic()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let workspace = TemporaryDirectory::new("publication")?;
    let provenance = workspace.path().join("publication-provenance.json");
    fs::write(
        &provenance,
        concat!(
            "{\n",
            "  \"server_tag\": \"v0.1.0\",\n",
            "  \"server_commit\": \"0123456789abcdef0123456789abcdef01234567\",\n",
            "  \"contract_version\": \"1\",\n",
            "  \"server_version\": \"0.1.0\"\n",
            "}\n"
        ),
    )?;
    let first = workspace.path().join("first");
    let second = workspace.path().join("second");
    contract_generation::generate(&first, &provenance)?;
    contract_generation::generate(&second, &provenance)?;
    for relative in contract_generation::expected_paths(&provenance)? {
        assert_eq!(
            fs::read(first.join(&relative))?,
            fs::read(second.join(&relative))?,
            "publication artifact differs: {relative}"
        );
    }
    let manifest: Value = serde_json::from_slice(&fs::read(first.join("manifest.json"))?)?;
    assert_eq!(
        manifest.get("server_commit").and_then(Value::as_str),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(manifest.get("server_tag").and_then(Value::as_str), Some("v0.1.0"));
    Ok(())
}

#[test]
fn committed_snapshot_matches_the_explicit_generator() -> Result<(), Box<dyn Error + Send + Sync>> {
    contract_generation::verify(&workspace_root().join("contracts"), &provenance_path())?;
    Ok(())
}

fn operation_ids(openapi: &Value) -> Result<BTreeSet<&str>, Box<dyn Error + Send + Sync>> {
    let paths = openapi
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| std::io::Error::other("OpenAPI paths are missing"))?;
    let mut operation_ids = BTreeSet::new();
    for path_item in paths.values() {
        let methods = path_item
            .as_object()
            .ok_or_else(|| std::io::Error::other("OpenAPI path item is invalid"))?;
        for operation in methods.values() {
            if let Some(operation_id) = operation.get("operationId").and_then(Value::as_str) {
                assert!(operation_ids.insert(operation_id), "duplicate operation ID");
            }
        }
    }
    Ok(operation_ids)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn provenance_path() -> PathBuf {
    workspace_root().join("src/contract_generation/provenance.json")
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> std::io::Result<Self> {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "jamye-contract-test-{}-{label}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
