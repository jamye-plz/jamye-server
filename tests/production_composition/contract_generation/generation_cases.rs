const EXPECTED_REALTIME_MAPPING: [RealtimeMappingOracle; 2] = [
    RealtimeMappingOracle::new(
        "message.created",
        1,
        "src/adapters/postgres/messaging/send.rs::persist_event_and_outbox",
        "tests/realtime/c1.rs::dev_c1_flows_from_seed_to_rest_outbox_redis_websocket_and_delta",
        "contracts/fixtures/message.created.json",
        "contracts/realtime/message.created.schema.json",
    ),
    RealtimeMappingOracle::new(
        "topic.created",
        1,
        "src/adapters/postgres/topics/mutation.rs::create_topic",
        "tests/topics/create.rs::t1_is_atomic_idempotent_and_emits_distinct_bootstrap_and_announcement_events",
        "contracts/contributions/task-7/fixtures/topic-flow.json",
        "contracts/realtime/topic.created.schema.json",
    ),
];

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
static FILESYSTEM_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ProvenanceInput {
    server_tag: Option<String>,
    server_commit: String,
    contract_version: String,
    server_version: String,
}

#[derive(Clone, Copy)]
struct RestMappingOracle {
    operation_id: &'static str,
    method: &'static str,
    path: &'static str,
    handler: &'static str,
    feature_behavior_test: &'static str,
    fixture: &'static str,
}

impl RestMappingOracle {
    const fn new(
        operation_id: &'static str,
        method: &'static str,
        path: &'static str,
        handler: &'static str,
        feature_behavior_test: &'static str,
        fixture: &'static str,
    ) -> Self {
        Self {
            operation_id,
            method,
            path,
            handler,
            feature_behavior_test,
            fixture,
        }
    }

    fn matches(self, row: &Value) -> bool {
        row.get("operation_id").and_then(Value::as_str) == Some(self.operation_id)
            && row.get("method").and_then(Value::as_str) == Some(self.method)
            && row.get("path").and_then(Value::as_str) == Some(self.path)
            && row.get("handler").and_then(Value::as_str) == Some(self.handler)
            && row.get("handler_route_probe").and_then(Value::as_str) == Some(ROUTE_PROBE)
            && row.get("feature_behavior_test").and_then(Value::as_str)
                == Some(self.feature_behavior_test)
            && row.get("fixture").and_then(Value::as_str) == Some(self.fixture)
    }
}

#[derive(Clone, Copy)]
struct RealtimeMappingOracle {
    event_type: &'static str,
    version: u64,
    handler: &'static str,
    feature_behavior_test: &'static str,
    fixture: &'static str,
    schema: &'static str,
}

impl RealtimeMappingOracle {
    const fn new(
        event_type: &'static str,
        version: u64,
        handler: &'static str,
        feature_behavior_test: &'static str,
        fixture: &'static str,
        schema: &'static str,
    ) -> Self {
        Self {
            event_type,
            version,
            handler,
            feature_behavior_test,
            fixture,
            schema,
        }
    }

    fn matches(self, row: &Value) -> bool {
        row.get("event_type").and_then(Value::as_str) == Some(self.event_type)
            && row.get("version").and_then(Value::as_u64) == Some(self.version)
            && row.get("handler").and_then(Value::as_str) == Some(self.handler)
            && row.get("handler_route_probe").and_then(Value::as_str) == Some(ROUTE_PROBE)
            && row.get("feature_behavior_test").and_then(Value::as_str)
                == Some(self.feature_behavior_test)
            && row.get("fixture").and_then(Value::as_str) == Some(self.fixture)
            && row.get("schema").and_then(Value::as_str) == Some(self.schema)
    }
}

#[test]
fn committed_explicit_provenance_inputs_are_parseable_and_locked() -> TestResult {
    let dirty = read_provenance(DIRTY)?;
    let clean_prepublication = read_provenance(CLEAN_PREPUBLICATION)?;
    let future_transition = read_provenance(FUTURE_TRANSITION)?;

    for input in [&dirty, &clean_prepublication] {
        assert_eq!(input.server_commit, "dirty");
        assert_eq!(input.server_tag, None);
        assert_eq!(input.contract_version, "1");
        assert_eq!(input.server_version, "0.1.0");
    }
    assert_eq!(
        future_transition.server_commit,
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(future_transition.server_tag.as_deref(), Some("v0.1.0"));
    assert_eq!(future_transition.contract_version, "1");
    assert_eq!(future_transition.server_version, "0.1.0");
    Ok(())
}

#[test]
fn generated_inventory_has_exactly_42_rest_operations_and_two_selected_realtime_events()
-> TestResult {
    let _filesystem = filesystem_lock();
    let generated = generate_current(DIRTY, "inventory")?;
    let openapi = read_json(&generated.path().join("openapi.json"))?;
    let actual_operations = openapi_operations(&openapi)?;
    let protocol = read_json(&generated.path().join("realtime/protocol.json"))?;
    let actual_events = protocol_events(&protocol)?;

    let expected_operations = EXPECTED_REST_OPERATIONS
        .into_iter()
        .collect::<BTreeSet<_>>();
    let expected_events = expected_realtime_event_types();
    if actual_operations != expected_operations || actual_events != expected_events {
        return Err(io::Error::other(format!(
            "Task-12 RED: generated selected inventory must contain the exact 42 operation/method/path rows and two unique selected realtime events; actual_rest_count={}, actual_rest={actual_operations:?}, actual_realtime_count={}, actual_realtime={actual_events:?}",
            actual_operations.len(),
            actual_events.len(),
        ))
        .into());
    }
    for (event_type, version) in EXPECTED_REALTIME_EVENTS {
        assert_realtime_event_schema(generated.path(), event_type, version)?;
    }
    Ok(())
}
#[test]
fn generated_selected_surface_mapping_has_one_handler_owner_test_and_fixture_per_operation_and_event()
-> TestResult {
    let _filesystem = filesystem_lock();
    let generated = generate_current(DIRTY, "surface-mapping")?;
    let mapping_path = generated
        .path()
        .join("fixtures/selected-surface-mapping.json");
    let mapping = fs::read(&mapping_path).map_err(|error| {
        io::Error::other(format!(
            "Task-12 RED: selected-surface mapping artifact is absent at {}: {error}",
            mapping_path.display()
        ))
    })?;
    let mapping: Value = serde_json::from_slice(&mapping)?;
    let mapping_fields = mapping
        .as_object()
        .ok_or_else(|| io::Error::other("Task-12 RED: selected-surface mapping must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    require_eq(
        mapping_fields,
        BTreeSet::from(["realtime_events", "rest_operations"]),
        "Task-12 RED: selected-surface mapping top-level fields changed",
    )?;

    assert_surface_rows(
        &mapping,
        "rest_operations",
        "operation_id",
        &expected_rest_operation_ids(),
    )?;
    assert_surface_rows(
        &mapping,
        "realtime_events",
        "event_type",
        &expected_realtime_event_types(),
    )?;
    let gap_fixture = read_json(
        &generated
            .path()
            .join("fixtures/c2-health-profile-account.json"),
    )?;
    require_eq(
        gap_fixture,
        serde_json::json!({
            "fixture": "c2_health_profile_account_selected_surfaces",
            "operation_ids": ["H1", "H2", "U1", "U2", "U3"],
            "evidence": "existing_owner_behavior_tests_named_by_selected-surface-mapping",
            "task_12_role": "declarative_mapping_gap_closure"
        }),
        "Task-12 RED: H1/H2/U1/U2/U3 must use the explicit C2 declarative gap fixture",
    )
}

#[test]
fn disposable_generation_rejects_unapproved_input_and_destination_before_writing() -> TestResult {
    let _filesystem = filesystem_lock();
    let mut failures = Vec::new();

    let (resolved_output, resolved_provenance) =
        contract_snapshot::validate_release_candidate_paths(
            Path::new("contracts"),
            Path::new("tests/production_composition/fixtures/contract_generation/dirty.json"),
        )?;
    require_eq(
        resolved_output,
        workspace_root().join("contracts"),
        "Task-12 public release-candidate relative output must resolve to the workspace contract tree",
    )?;
    require_eq(
        resolved_provenance,
        provenance_path(DIRTY),
        "Task-12 public release-candidate relative provenance must resolve to the workspace fixture",
    )?;

    let public_release_candidate_destination =
        OwnedDirectory::new(DIRTY, "public-release-candidate-output")?;
    record_rejection(
        &mut failures,
        "public release-candidate disposable destination",
        contract_snapshot::generate_release_candidate(
            public_release_candidate_destination.path(),
            &provenance_path(DIRTY),
        ),
        public_release_candidate_destination.path(),
    )?;

    let unapproved_input = OwnedDirectory::new("unapproved-input", "provenance")?;
    let unapproved_provenance = unapproved_input.path().join("input.json");
    fs::write(&unapproved_provenance, fs::read(provenance_path(DIRTY))?)?;
    let allowed_destination = OwnedDirectory::new(DIRTY, "invalid-input-output")?;
    record_rejection(
        &mut failures,
        "unapproved provenance",
        contract_snapshot::generate_disposable(allowed_destination.path(), &unapproved_provenance),
        allowed_destination.path(),
    )?;

    let cross_pair_destination =
        OwnedDirectory::new(CLEAN_PREPUBLICATION, "dirty-cross-pair-output")?;
    record_rejection(
        &mut failures,
        "mismatched provenance and fixture destination",
        contract_snapshot::generate_disposable(
            cross_pair_destination.path(),
            &provenance_path(DIRTY),
        ),
        cross_pair_destination.path(),
    )?;

    let contribution_destination =
        OwnedDirectory::new(DIRTY, "preseeded-owner-contributions-output")?;
    let copied_contributions = copy_owner_contribution_tree(contribution_destination.path())?;
    require_eq(
        copied_contributions,
        18,
        "Task-12 owner contribution fixture count changed",
    )?;
    let contribution_before = directory_snapshot(contribution_destination.path())?;
    let contribution_result = contract_snapshot::generate_disposable(
        contribution_destination.path(),
        &provenance_path(DIRTY),
    );
    let contribution_after = directory_snapshot(contribution_destination.path())?;
    if contribution_result.is_ok() || contribution_before != contribution_after {
        failures.push(format!(
            "pre-existing owner contributions: accepted={}, changed={}",
            contribution_result.is_ok(),
            contribution_before != contribution_after
        ));
    }

    let outside_destination = OwnedOutsideDirectory::new("outside-output")?;
    record_rejection(
        &mut failures,
        "destination outside the task root",
        contract_snapshot::generate_disposable(outside_destination.path(), &provenance_path(DIRTY)),
        outside_destination.path(),
    )?;

    let traversal_target = OwnedDirectory::new(CLEAN_PREPUBLICATION, "traversal-target")?;
    let traversal_destination = generation_root()
        .join(DIRTY)
        .join("..")
        .join(CLEAN_PREPUBLICATION)
        .join(
            traversal_target
                .path()
                .file_name()
                .ok_or_else(|| io::Error::other("Task-12 traversal target has no file name"))?,
        );
    record_rejection(
        &mut failures,
        "destination containing parent traversal",
        contract_snapshot::generate_disposable(
            &traversal_destination,
            &provenance_path(CLEAN_PREPUBLICATION),
        ),
        traversal_target.path(),
    )?;

    let symlink_input_owner = OwnedDirectory::new("unapproved-input", "symlink-owner")?;
    let symlink_input = OwnedSymlink::new(
        symlink_input_owner.path().join("provenance-link.json"),
        provenance_path(DIRTY),
    )?;
    let symlink_input_destination = OwnedDirectory::new(DIRTY, "symlink-input-output")?;
    record_rejection(
        &mut failures,
        "symlink provenance",
        contract_snapshot::generate_disposable(
            symlink_input_destination.path(),
            symlink_input.path(),
        ),
        symlink_input_destination.path(),
    )?;

    let symlink_output_target = OwnedOutsideDirectory::new("symlink-output-target")?;
    let symlink_output = OwnedSymlink::new(
        generation_root()
            .join(DIRTY)
            .join(unique_leaf("output-link")),
        symlink_output_target.path().to_owned(),
    )?;
    record_rejection(
        &mut failures,
        "symlink destination",
        contract_snapshot::generate_disposable(symlink_output.path(), &provenance_path(DIRTY)),
        symlink_output_target.path(),
    )?;
    if !failures.is_empty() {
        return Err(io::Error::other(format!(
            "Task-12 RED: disposable generation failed its pre-write allowlist boundary: {}",
            failures.join("; ")
        ))
        .into());
    }
    Ok(())
}

#[test]
fn allowed_provenance_variants_are_deterministic_and_emit_release_candidate_manifest() -> TestResult
{
    let _filesystem = filesystem_lock();
    assert_release_candidate_path_partition()?;

    let dirty_prepublication = generate_current(DIRTY, "prepublication-dirty")?;
    let clean_prepublication = generate_current(CLEAN_PREPUBLICATION, "prepublication-clean")?;
    let dirty_manifest = read_json(&dirty_prepublication.path().join("manifest.json"))?;
    let clean_manifest = read_json(&clean_prepublication.path().join("manifest.json"))?;
    assert_release_candidate_artifact_allowlist(&dirty_manifest, "dirty prepublication")?;
    assert_release_candidate_artifact_allowlist(&clean_manifest, "clean prepublication")?;
    for relative in EXPECTED_RELEASE_CANDIDATE_ARTIFACTS {
        require_eq(
            fs::read(dirty_prepublication.path().join(relative))?,
            fs::read(clean_prepublication.path().join(relative))?,
            &format!(
                "Task-12 RED: dirty and clean-prepublication inputs changed bytes for {relative}"
            ),
        )?;
    }

    let committed_contracts = workspace_root().join("contracts");
    let committed_manifest = read_json(&committed_contracts.join("manifest.json"))?;
    assert_release_candidate_artifact_allowlist(&committed_manifest, "committed contracts")?;
    require_eq(
        directory_snapshot(&committed_contracts.join("contributions"))?.len(),
        18,
        "Task-12 committed release candidate must retain the exact owner contribution file count",
    )?;
    contract_snapshot::verify(&committed_contracts, &provenance_path(DIRTY))?;

    for case in CASES {
        let first = generate_current(case, "deterministic-first")?;
        let second = generate_current(case, "deterministic-second")?;
        let manifest = read_json(&first.path().join("manifest.json"))?;
        let second_manifest = read_json(&second.path().join("manifest.json"))?;
        assert_release_candidate_artifact_allowlist(&manifest, case)?;
        assert_release_candidate_artifact_allowlist(&second_manifest, case)?;
        for relative in EXPECTED_RELEASE_CANDIDATE_ARTIFACTS {
            require_eq(
                fs::read(first.path().join(relative))?,
                fs::read(second.path().join(relative))?,
                &format!("Task-12 RED: {case} generation changed bytes for {relative}"),
            )?;
        }

        require_eq(
            manifest.get("stage").and_then(Value::as_str),
            Some("release_candidate"),
            &format!("Task-12 RED: {case} manifest must declare stage=release_candidate"),
        )?;
        assert_manifest_provenance(case, &manifest)?;
        assert_checksum_validation(first.path(), case)?;
    }
    Ok(())
}
