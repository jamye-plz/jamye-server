use std::{error::Error, fs, io};

#[path = "topics/contract.rs"]
mod contract;
#[path = "topics/create.rs"]
mod create;
#[path = "topics/helpers.rs"]
mod topic_helpers;
#[path = "topics/http.rs"]
mod http;
#[path = "topics/migration.rs"]
mod migration;
#[path = "topics/pagination.rs"]
mod pagination;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "topics/update.rs"]
mod update;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[test]
fn production_topics_surface_is_statically_registered() -> io::Result<()> {
    for path in [
        "migrations/0005_topics.sql",
        "src/application/topics/mod.rs",
        "src/ports/topics/mod.rs",
        "src/adapters/postgres/topics/mod.rs",
        "src/adapters/postgres/topics/query.rs",
        "src/adapters/postgres/topics/mutation.rs",
        "src/transport/http/topics/mod.rs",
        "contracts/contributions/task-7/dto/operations.json",
        "contracts/contributions/task-7/schemas/topics-wire.schema.json",
        "contracts/contributions/task-7/fixtures/topic-flow.json",
        "docs/commands/task-7/topics.md",
        "scripts/tasks/task-7/mod.just",
    ] {
        assert!(fs::metadata(path)?.is_file(), "missing task-7 surface: {path}");
    }

    let application = fs::read_to_string("src/application/topics/mod.rs")?;
    let repository = fs::read_to_string("src/adapters/postgres/topics/mod.rs")?
        + &fs::read_to_string("src/adapters/postgres/topics/query.rs")?
        + &fs::read_to_string("src/adapters/postgres/topics/mutation.rs")?;
    let transport = fs::read_to_string("src/transport/http/topics/mod.rs")?;
    assert!(!application.contains("sqlx::"));
    assert!(!application.contains("crate::adapters"));
    assert!(!application.contains("crate::transport"));
    for forbidden in [
        ".begin()",
        ".commit()",
        ".rollback()",
        "UnitOfWork",
        "registry",
        "plugin",
        "hook",
    ] {
        assert!(
            !repository.contains(forbidden),
            "repository owns forbidden transaction/discovery behavior: {forbidden}"
        );
    }
    for route in [
        "/api/v1/groups/{group_id}/topics",
        "/api/v1/groups/{group_id}/topics/dates",
        "/api/v1/groups/{group_id}/topics/{topic_id}",
        "/api/v1/groups/{group_id}/topics/{topic_id}/tags",
    ] {
        assert!(transport.contains(route), "missing task-7 route: {route}");
    }
    Ok(())
}
