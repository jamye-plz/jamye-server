fn assert_release_candidate_path_partition() -> TestResult {
    let c0_paths = contract_snapshot::expected_paths(&provenance_path(DIRTY))?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let release_candidate_paths = EXPECTED_RELEASE_CANDIDATE_ARTIFACTS
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();
    let release_candidate_only = release_candidate_paths
        .difference(&c0_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_release_candidate_only = EXPECTED_RELEASE_CANDIDATE_ONLY_ARTIFACTS
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();

    require_eq(
        c0_paths.len(),
        16,
        "Task-12 preserved C0 artifact count changed",
    )?;
    require_eq(
        release_candidate_paths.len(),
        21,
        "Task-12 release-candidate artifact count changed",
    )?;
    require_eq(
        release_candidate_only,
        expected_release_candidate_only,
        "Task-12 release candidate must add exactly the five selected C2 artifacts",
    )
}

fn assert_release_candidate_artifact_allowlist(manifest: &Value, label: &str) -> TestResult {
    let artifacts = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::other(format!("Task-12 {label} manifest lacks artifacts")))?;
    let actual = artifacts
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            io::Error::other(format!(
                "Task-12 {label} manifest contains a non-string artifact path"
            ))
        })?;
    require_eq(
        actual,
        EXPECTED_RELEASE_CANDIDATE_ARTIFACTS.to_vec(),
        &format!("Task-12 {label} manifest must declare the exact 21 release-candidate artifacts"),
    )?;
    require_eq(
        manifest.get("stage").and_then(Value::as_str),
        Some("release_candidate"),
        &format!("Task-12 {label} manifest must declare stage=release_candidate"),
    )
}

#[test]
fn generated_c2_mobile_handoff_preserves_the_exact_sqlite_auth_and_delta_contract() -> TestResult {
    let _filesystem = filesystem_lock();
    let generated = generate_current(DIRTY, "mobile-handoff")?;
    let path = generated.path().join("fixtures/c2-mobile-handoff.json");
    let handoff = read_required_fixture(&path, "C2 mobile handoff")?;

    require_eq(
        handoff.get("atomicity_contract").and_then(Value::as_str),
        Some(contract_snapshot::fixtures::SQLITE_ATOMICITY_SENTENCE),
        "Task-12 RED: C2 mobile handoff must repeat the canonical SQLite sentence exactly",
    )?;
    require_eq(
        handoff
            .pointer("/references/task_5_auth_trace")
            .and_then(Value::as_str),
        Some("contracts/contributions/task-5/fixtures/mobile-auth-handoff.json"),
        "Task-12 RED: C2 mobile handoff must reference Task-5's sole auth trace",
    )?;
    require_eq(
        handoff
            .pointer("/references/task_3b_task_4b_two_phase_delta")
            .and_then(Value::as_str),
        Some(
            "contracts/fixtures/mobile-sync-handoff.json;tests/realtime/c1.rs::dev_c1_flows_from_seed_to_rest_outbox_redis_websocket_and_delta",
        ),
        "Task-12 RED: C2 mobile handoff must reference, not reimplement, the fully drained two-phase delta evidence",
    )?;
    require_eq(
        handoff.get("execution_owner").and_then(Value::as_str),
        Some("jamye-app"),
        "Task-12 RED: executable mobile SQLite behavior remains jamye-app-owned",
    )?;
    require_eq(
        handoff.get("server_execution").and_then(Value::as_str),
        Some("none"),
        "Task-12 RED: server must not claim executable mobile SQLite behavior",
    )
}

#[test]
fn generated_bodyless_audio_reachability_traces_retry_delivery_history_and_metadata_only_reissue()
-> TestResult {
    let _filesystem = filesystem_lock();
    let generated = generate_current(DIRTY, "bodyless-audio")?;
    let path = generated
        .path()
        .join("fixtures/c2-bodyless-audio-reachability.json");
    let reachability = read_required_fixture(&path, "C2 bodyless-audio reachability")?;

    require_eq(
        reachability
            .pointer("/c4/request/body")
            .is_some_and(Value::is_null),
        true,
        "Task-12 RED: C2 reachability must begin with a bodyless C4 request",
    )?;
    require_eq(
        reachability
            .pointer("/c4/request/media")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "Task-12 RED: C2 reachability must use exactly one finalized audio upload",
    )?;
    require_eq(
        reachability
            .pointer("/c4/request/media/0/state")
            .and_then(Value::as_str),
        Some("finalized"),
        "Task-12 RED: C2 reachability must use a finalized upload",
    )?;
    require_eq(
        reachability
            .pointer("/c4/request/media/0/mime_type")
            .and_then(Value::as_str),
        Some("audio/m4a"),
        "Task-12 RED: C2 reachability must use an audio upload",
    )?;
    require_eq(
        reachability.pointer("/c4/request/client_msg_id"),
        reachability.pointer("/c4/retry/client_msg_id"),
        "Task-12 RED: bodyless C4 retry must retain client_msg_id",
    )?;
    require_eq(
        reachability
            .pointer("/server_path/composition")
            .and_then(Value::as_str),
        Some("SendMessage"),
        "Task-12 RED: bodyless C4 must pass through the Sprint-2 SendMessage composition",
    )?;
    require_eq(
        reachability
            .pointer("/server_path/delivery")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
            }),
        Some(BTreeSet::from([
            "worker",
            "Redis",
            "WebSocket message.created",
            "offline delta",
            "history",
            "MD4 metadata-only reissue",
            "MD5 metadata-only reissue",
        ])),
        "Task-12 RED: bodyless audio reachability must cover worker/Redis/WebSocket, offline delta/history, and metadata-only MD4/MD5 reissue",
    )?;
    require_eq(
        reachability
            .pointer("/mobile_execution/playback")
            .and_then(Value::as_str),
        Some("jamye-app-owned"),
        "Task-12 RED: C2 reachability must not claim executable mobile playback",
    )?;
    require_eq(
        reachability
            .pointer("/mobile_execution/server_execution")
            .and_then(Value::as_str),
        Some("none"),
        "Task-12 RED: C2 reachability must not claim server-side mobile execution",
    )
}

fn assert_surface_rows(
    mapping: &Value,
    collection: &str,
    identifier: &str,
    expected: &BTreeSet<&str>,
) -> TestResult {
    let rows = mapping
        .get(collection)
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::other(format!("Task-12 RED: mapping {collection} is missing")))?;
    let mut actual = BTreeSet::new();
    for row in rows {
        let id = row.get(identifier).and_then(Value::as_str).ok_or_else(|| {
            io::Error::other(format!(
                "Task-12 RED: mapping {collection} lacks {identifier}"
            ))
        })?;
        if !actual.insert(id) {
            return Err(io::Error::other(format!(
                "Task-12 RED: mapping {collection} duplicates {identifier}={id}"
            ))
            .into());
        }
        assert_surface_identity(row, collection, identifier, id)?;
        assert_exact_mapping_row(row, collection, identifier, id)?;
        let handler = required_mapping_field(row, collection, identifier, id, "handler")?;
        let handler_route_probe =
            required_mapping_field(row, collection, identifier, id, "handler_route_probe")?;
        let feature_behavior_test =
            required_mapping_field(row, collection, identifier, id, "feature_behavior_test")?;
        let fixture = required_mapping_field(row, collection, identifier, id, "fixture")?;
        require_repository_symbol_reference(handler, collection, identifier, id, "handler")?;
        require_repository_symbol_reference(
            handler_route_probe,
            collection,
            identifier,
            id,
            "handler_route_probe",
        )?;
        require_repository_symbol_reference(
            feature_behavior_test,
            collection,
            identifier,
            id,
            "feature_behavior_test",
        )?;
        require_safe_repository_file(fixture, collection, identifier, id, "fixture")?;
        if collection == "realtime_events" {
            let schema = required_mapping_field(row, collection, identifier, id, "schema")?;
            require_safe_repository_file(schema, collection, identifier, id, "schema")?;
        }
    }
    require_eq(
        actual,
        expected.clone(),
        &format!("Task-12 RED: mapping {collection} does not cover the exact selected inventory"),
    )
}

fn assert_exact_mapping_row(
    row: &Value,
    collection: &str,
    identifier: &str,
    id: &str,
) -> TestResult {
    let expected_fields = match collection {
        "rest_operations" => REST_MAPPING_FIELDS.as_slice(),
        "realtime_events" => REALTIME_MAPPING_FIELDS.as_slice(),
        _ => unreachable!("collection was validated by assert_surface_identity"),
    };
    let actual_fields = row
        .as_object()
        .ok_or_else(|| io::Error::other("Task-12 RED: mapping row must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_fields = expected_fields.iter().copied().collect::<BTreeSet<_>>();
    if actual_fields != expected_fields {
        return Err(io::Error::other(format!(
            "Task-12 RED: mapping {collection} {identifier}={id} fields differ: actual={actual_fields:?}, expected={expected_fields:?}"
        ))
        .into());
    }

    let matches = match collection {
        "rest_operations" => EXPECTED_REST_MAPPING
            .iter()
            .copied()
            .any(|expected| expected.matches(row)),
        "realtime_events" => EXPECTED_REALTIME_MAPPING
            .iter()
            .copied()
            .any(|expected| expected.matches(row)),
        _ => unreachable!("collection was validated by assert_surface_identity"),
    };
    if !matches {
        return Err(io::Error::other(format!(
            "Task-12 RED: mapping {collection} {identifier}={id} is not the exact approved handler/probe/owner-test/fixture record"
        ))
        .into());
    }
    Ok(())
}

fn assert_surface_identity(
    row: &Value,
    collection: &str,
    identifier: &str,
    id: &str,
) -> TestResult {
    match collection {
        "rest_operations" => {
            let method = required_mapping_field(row, collection, identifier, id, "method")?;
            let path = required_mapping_field(row, collection, identifier, id, "path")?;
            if !EXPECTED_REST_OPERATIONS.contains(&(id, method, path)) {
                return Err(io::Error::other(format!(
                    "Task-12 RED: REST mapping row is not the frozen operation/method/path triple: {id} {method} {path}"
                ))
                .into());
            }
        }
        "realtime_events" => {
            let version = row.get("version").and_then(Value::as_u64).ok_or_else(|| {
                io::Error::other(format!(
                    "Task-12 RED: realtime mapping event_type={id} lacks numeric version"
                ))
            })?;
            if !EXPECTED_REALTIME_EVENTS.contains(&(id, version)) {
                return Err(io::Error::other(format!(
                    "Task-12 RED: realtime mapping must contain only {id} version 1"
                ))
                .into());
            }
        }
        _ => {
            return Err(io::Error::other(format!(
                "Task-12 RED: unknown selected-surface collection {collection}"
            ))
            .into());
        }
    }
    Ok(())
}

fn assert_manifest_provenance(case: &str, manifest: &Value) -> TestResult {
    let expected = read_provenance(case)?;
    require_eq(
        manifest.get("server_commit").and_then(Value::as_str),
        Some(expected.server_commit.as_str()),
        &format!("Task-12 RED: {case} manifest changed explicit server_commit"),
    )?;
    let expected_tag = serde_json::to_value(expected.server_tag)?;
    require_eq(
        manifest.get("server_tag"),
        Some(&expected_tag),
        &format!("Task-12 RED: {case} manifest changed explicit server_tag"),
    )?;
    require_eq(
        manifest.get("contract_version").and_then(Value::as_str),
        Some(expected.contract_version.as_str()),
        &format!("Task-12 RED: {case} manifest changed explicit contract_version"),
    )?;
    require_eq(
        manifest.get("server_version").and_then(Value::as_str),
        Some(expected.server_version.as_str()),
        &format!("Task-12 RED: {case} manifest changed explicit server_version"),
    )?;
    require_eq(
        manifest.get("checksum_algorithm").and_then(Value::as_str),
        Some(
            "sha256 over lexicographic path,NUL,decimal-length,NUL,bytes entries; manifest.json uses recursively key-sorted compact JSON without sha256; v1",
        ),
        &format!("Task-12 RED: {case} manifest checksum must exclude its own sha256 field"),
    )?;
    let checksum = manifest
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::other("Task-12 RED: release manifest lacks sha256"))?;
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::other(
            "Task-12 RED: release manifest sha256 must be exactly 64 hexadecimal digits",
        )
        .into());
    }
    Ok(())
}

fn assert_checksum_validation(output: &Path, case: &str) -> TestResult {
    let provenance = provenance_path(case);
    contract_snapshot::verify(output, &provenance)?;

    let manifest_path = output.join("manifest.json");
    let original = fs::read(&manifest_path)?;
    let mut tampered: Value = serde_json::from_slice(&original)?;
    tampered["sha256"] = Value::String("0".repeat(64));
    let mut tampered_bytes = serde_json::to_vec_pretty(&tampered)?;
    tampered_bytes.push(b'\n');
    fs::write(&manifest_path, tampered_bytes)?;
    let rejected = contract_snapshot::verify(output, &provenance).is_err();
    fs::write(&manifest_path, original)?;

    require_eq(
        rejected,
        true,
        &format!("Task-12 RED: {case} manifest verification accepted a changed sha256"),
    )?;
    contract_snapshot::verify(output, &provenance)?;
    Ok(())
}

fn required_mapping_field<'a>(
    row: &'a Value,
    collection: &str,
    identifier: &str,
    id: &str,
    field: &str,
) -> TestResult<&'a str> {
    row.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            io::Error::other(format!(
                "Task-12 RED: mapping {collection} {identifier}={id} lacks one nonempty {field}"
            ))
            .into()
        })
}

fn require_repository_symbol_reference(
    reference: &str,
    collection: &str,
    identifier: &str,
    id: &str,
    field: &str,
) -> TestResult {
    let (file, symbol) = reference.rsplit_once("::").ok_or_else(|| {
        io::Error::other(format!(
            "Task-12 RED: mapping {collection} {identifier}={id} {field} must be a repository-relative file::symbol reference"
        ))
    })?;
    if symbol.is_empty() {
        return Err(io::Error::other(format!(
            "Task-12 RED: mapping {collection} {identifier}={id} {field} has an empty symbol: {reference}"
        ))
        .into());
    }
    require_safe_repository_file(file, collection, identifier, id, field)
}
