fn require_safe_repository_file(
    reference: &str,
    collection: &str,
    identifier: &str,
    id: &str,
    field: &str,
) -> TestResult {
    let relative = Path::new(reference);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::other(format!(
            "Task-12 RED: mapping {collection} {identifier}={id} {field} is not a safe repository-relative path: {reference}"
        ))
        .into());
    }

    let mut current = workspace_root();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        let Component::Normal(component) = component else {
            unreachable!("components were validated above")
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            io::Error::other(format!(
                "Task-12 RED: mapping {collection} {identifier}={id} {field} is absent at {}: {error}",
                current.display()
            ))
        })?;
        if metadata.file_type().is_symlink()
            || (index + 1 == component_count && !metadata.is_file())
            || (index + 1 < component_count && !metadata.is_dir())
        {
            return Err(io::Error::other(format!(
                "Task-12 RED: mapping {collection} {identifier}={id} {field} traverses a symlink or non-file path: {reference}"
            ))
            .into());
        }
    }
    Ok(())
}

fn openapi_operations(openapi: &Value) -> TestResult<BTreeSet<(&str, &str, &str)>> {
    let paths = openapi
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| io::Error::other("Task-12 RED: OpenAPI paths are missing"))?;
    let mut operations = BTreeSet::new();
    for (path, path_item) in paths {
        let methods = path_item
            .as_object()
            .ok_or_else(|| io::Error::other("Task-12 RED: OpenAPI path item is invalid"))?;
        for (method, operation) in methods {
            if !is_http_method(method) {
                continue;
            }
            let id = operation
                .get("operationId")
                .and_then(Value::as_str)
                .ok_or_else(|| io::Error::other("Task-12 RED: OpenAPI operation ID is missing"))?;
            if !operations.insert((id, method.as_str(), path.as_str())) {
                return Err(io::Error::other(format!(
                    "Task-12 RED: duplicate generated operation row {id} {method} {path}"
                ))
                .into());
            }
        }
    }
    Ok(operations)
}

fn is_http_method(value: &str) -> bool {
    matches!(
        value,
        "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace"
    )
}

fn protocol_events(protocol: &Value) -> TestResult<BTreeSet<&str>> {
    let events = protocol
        .get("known_event_discriminants")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            io::Error::other("Task-12 RED: realtime protocol event inventory is missing")
        })?;
    let mut unique = BTreeSet::new();
    for event in events {
        let event = event
            .as_str()
            .ok_or_else(|| io::Error::other("Task-12 RED: realtime event must be a string"))?;
        if !unique.insert(event) {
            return Err(io::Error::other(format!(
                "Task-12 RED: realtime protocol duplicates {event}"
            ))
            .into());
        }
    }
    Ok(unique)
}

fn assert_realtime_event_schema(output: &Path, event_type: &str, version: u64) -> TestResult {
    let schema = read_json(&output.join(format!("realtime/{event_type}.schema.json")))?;
    let expected_id = format!("https://contracts.jamye.local/realtime/{event_type}.schema.json");
    require_eq(
        schema.get("$id").and_then(Value::as_str),
        Some(expected_id.as_str()),
        &format!("Task-12 RED: {event_type} schema ID changed"),
    )?;
    require_eq(
        schema
            .pointer("/properties/version/minimum")
            .and_then(Value::as_u64),
        Some(version),
        &format!("Task-12 RED: {event_type} minimum version changed"),
    )?;
    require_eq(
        schema
            .pointer("/properties/version/maximum")
            .and_then(Value::as_u64),
        Some(version),
        &format!("Task-12 RED: {event_type} maximum version changed"),
    )?;

    let discriminator = schema.pointer("/properties/type").ok_or_else(|| {
        io::Error::other(format!(
            "Task-12 RED: {event_type} schema type discriminator is missing"
        ))
    })?;
    let discriminator = match discriminator.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let pointer = reference.strip_prefix('#').ok_or_else(|| {
                io::Error::other(format!(
                    "Task-12 RED: {event_type} schema type discriminator must use a local reference"
                ))
            })?;
            schema.pointer(pointer).ok_or_else(|| {
                io::Error::other(format!(
                    "Task-12 RED: {event_type} schema type discriminator reference is unresolved"
                ))
            })?
        }
        None => discriminator,
    };
    let exact_discriminator = discriminator.get("const").and_then(Value::as_str)
        == Some(event_type)
        || discriminator
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| values.len() == 1 && values[0].as_str() == Some(event_type));
    require_eq(
        exact_discriminator,
        true,
        &format!(
            "Task-12 RED: {event_type} schema type discriminator must be exactly {event_type}"
        ),
    )?;
    require_eq(
        schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.iter().any(|field| field.as_str() == Some("type"))),
        true,
        &format!("Task-12 RED: {event_type} schema must require its type discriminator"),
    )
}

fn expected_rest_operation_ids() -> BTreeSet<&'static str> {
    EXPECTED_REST_OPERATIONS
        .iter()
        .map(|(operation_id, _, _)| *operation_id)
        .collect()
}

fn expected_realtime_event_types() -> BTreeSet<&'static str> {
    EXPECTED_REALTIME_EVENTS
        .iter()
        .map(|(event_type, _)| *event_type)
        .collect()
}

fn read_required_fixture(path: &Path, label: &str) -> TestResult<Value> {
    let bytes = fs::read(path).map_err(|error| {
        io::Error::other(format!(
            "Task-12 RED: generated {label} fixture is absent at {}: {error}",
            path.display()
        ))
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn read_json(path: &Path) -> TestResult<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn read_provenance(case: &str) -> TestResult<ProvenanceInput> {
    Ok(serde_json::from_slice(&fs::read(provenance_path(case))?)?)
}

fn provenance_path(case: &str) -> PathBuf {
    workspace_root()
        .join("tests/production_composition/fixtures/contract_generation")
        .join(format!("{case}.json"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn generation_root() -> PathBuf {
    std::env::temp_dir().join("jamye-task-12-contract-generation")
}

fn generate_current(case: &str, label: &str) -> TestResult<OwnedDirectory> {
    let output = OwnedDirectory::new(case, label)?;
    contract_snapshot::generate_disposable(output.path(), &provenance_path(case))?;
    Ok(output)
}

fn contains_files(root: &Path) -> TestResult<bool> {
    Ok(fs::read_dir(root)?.next().transpose()?.is_some())
}

fn copy_owner_contribution_tree(destination: &Path) -> TestResult<usize> {
    fn copy_directory(source: &Path, destination: &Path) -> TestResult<usize> {
        fs::create_dir_all(destination)?;
        let mut copied = 0;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            let target = destination.join(entry.file_name());
            if metadata.file_type().is_symlink() {
                return Err(io::Error::other(
                    "owner contribution input must not contain a symlink",
                )
                .into());
            }
            if metadata.is_dir() {
                copied += copy_directory(&entry.path(), &target)?;
            } else if metadata.is_file() {
                fs::copy(entry.path(), target)?;
                copied += 1;
            } else {
                return Err(io::Error::other(
                    "owner contribution input must contain only files and directories",
                )
                .into());
            }
        }
        Ok(copied)
    }

    copy_directory(
        &workspace_root().join("contracts/contributions"),
        &destination.join("contributions"),
    )
}

fn directory_snapshot(root: &Path) -> TestResult<Vec<(String, Vec<u8>)>> {
    fn collect(root: &Path, directory: &Path, files: &mut Vec<(String, Vec<u8>)>) -> TestResult {
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(io::Error::other("snapshot input must not contain a symlink").into());
            }
            if metadata.is_dir() {
                collect(root, &path, files)?;
            } else if metadata.is_file() {
                files.push((
                    path.strip_prefix(root)?.to_string_lossy().into_owned(),
                    fs::read(path)?,
                ));
            } else {
                return Err(io::Error::other(
                    "snapshot input must contain only files and directories",
                )
                .into());
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    Ok(files)
}

fn record_rejection<E>(
    failures: &mut Vec<String>,
    label: &str,
    result: Result<usize, E>,
    observed_output: &Path,
) -> TestResult {
    let wrote = contains_files(observed_output)?;
    if result.is_ok() || wrote {
        failures.push(format!(
            "{label}: accepted={}, wrote={wrote}",
            result.is_ok()
        ));
    }
    Ok(())
}

fn filesystem_lock() -> std::sync::MutexGuard<'static, ()> {
    FILESYSTEM_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn require_eq<T>(actual: T, expected: T, message: &str) -> TestResult
where
    T: std::fmt::Debug + PartialEq,
{
    if actual == expected {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{message}: actual={actual:?}, expected={expected:?}"
        ))
        .into())
    }
}

struct OwnedDirectory {
    path: PathBuf,
}

impl OwnedDirectory {
    fn new(case: &str, label: &str) -> TestResult<Self> {
        if !is_single_normal_component(case) || !is_single_normal_component(label) {
            return Err(io::Error::other(
                "Task-12 temporary case and label must each be one normal path component",
            )
            .into());
        }
        let root = generation_root();
        ensure_real_directory(&root)?;
        let case_root = root.join(case);
        ensure_real_directory(&case_root)?;
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let path = case_root.join(format!("{label}-{}-{sequence}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
fn unique_leaf(label: &str) -> String {
    let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    format!("{label}-{}-{sequence}", std::process::id())
}

fn is_single_normal_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn is_owned_child(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        let mut components = relative.components();
        matches!(components.next(), Some(Component::Normal(_)))
            && components.all(|component| matches!(component, Component::Normal(_)))
    })
}

fn ensure_real_directory(path: &Path) -> TestResult {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(io::Error::other(format!(
            "Task-12 owned directory is a symlink or non-directory: {}",
            path.display()
        ))
        .into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        if is_owned_child(&self.path, &generation_root()) {
            let _ = fs::remove_dir_all(&self.path);
            if let Some(case_root) = self.path.parent() {
                let _ = fs::remove_dir(case_root);
            }
            let _ = fs::remove_dir(generation_root());
        }
    }
}

struct OwnedOutsideDirectory {
    path: PathBuf,
}

impl OwnedOutsideDirectory {
    fn new(label: &str) -> TestResult<Self> {
        if !is_single_normal_component(label) {
            return Err(io::Error::other(
                "Task-12 outside-directory label must be one normal path component",
            )
            .into());
        }
        let path = std::env::temp_dir().join(format!(
            "jamye-task-12-contract-generation-outside-{}",
            unique_leaf(label)
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
