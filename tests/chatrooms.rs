use std::{error::Error, fs, io};

#[path = "chatrooms/helpers.rs"]
mod chatroom_helpers;
#[path = "chatrooms/contract.rs"]
mod contract;
#[path = "chatrooms/http.rs"]
mod http;
#[path = "chatrooms/migration.rs"]
mod migration;
#[path = "chatrooms/pagination.rs"]
mod pagination;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "chatrooms/read.rs"]
mod read;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[test]
fn production_chatrooms_surface_is_statically_registered() -> io::Result<()> {
    for path in [
        "migrations/0004_chatroom_reads.sql",
        "src/application/chatrooms/mod.rs",
        "src/ports/chatrooms/mod.rs",
        "src/adapters/postgres/chatrooms/mod.rs",
        "src/adapters/postgres/chatrooms/query.rs",
        "src/adapters/postgres/chatrooms/mutation.rs",
        "src/transport/http/chatrooms/mod.rs",
        "contracts/contributions/task-6b/dto/operations.json",
        "contracts/contributions/task-6b/schemas/chatrooms-wire.schema.json",
        "contracts/contributions/task-6b/fixtures/chatroom-history-read.json",
        "docs/commands/task-6b/chatrooms.md",
        "scripts/tasks/task-6b/mod.just",
    ] {
        assert!(
            fs::metadata(path)?.is_file(),
            "missing task-6b surface: {path}"
        );
    }

    let application = fs::read_to_string("src/application/chatrooms/mod.rs")?;
    let repository = fs::read_to_string("src/adapters/postgres/chatrooms/mod.rs")?
        + &fs::read_to_string("src/adapters/postgres/chatrooms/query.rs")?
        + &fs::read_to_string("src/adapters/postgres/chatrooms/mutation.rs")?;
    let transport = fs::read_to_string("src/transport/http/chatrooms/mod.rs")?;
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
        "/api/v1/groups/{group_id}/chatrooms",
        "/api/v1/chatrooms/{chatroom_id}/messages",
        "/api/v1/chatrooms/{chatroom_id}/read",
    ] {
        assert!(transport.contains(route), "missing task-6b route: {route}");
    }
    Ok(())
}
