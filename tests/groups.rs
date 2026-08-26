use std::{error::Error, fs, io};

#[path = "groups/contract.rs"]
mod contract;
#[path = "groups/helpers.rs"]
mod groups_helpers;
#[path = "groups/http.rs"]
mod http;
#[path = "groups/invites.rs"]
mod invites;
#[path = "groups/migration.rs"]
mod migration;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "groups/rate_limit.rs"]
mod rate_limit;
#[path = "groups/redis_recovery.rs"]
mod redis_recovery;
#[path = "groups/topology.rs"]
mod topology;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[test]
fn production_groups_surface_is_statically_registered() -> io::Result<()> {
    for path in [
        "migrations/0003_invites.sql",
        "src/application/groups/mod.rs",
        "src/ports/groups/mod.rs",
        "src/adapters/postgres/groups/mod.rs",
        "src/adapters/postgres/groups/query.rs",
        "src/adapters/postgres/groups/mutation.rs",
        "src/transport/http/groups/mod.rs",
        "contracts/contributions/task-6/dto/operations.json",
        "contracts/contributions/task-6/schemas/groups-wire.schema.json",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json",
        "docs/commands/task-6/groups.md",
        "scripts/tasks/task-6/mod.just",
        "scripts/tasks/task-6/redis-recovery.sh",
    ] {
        assert!(
            fs::metadata(path)?.is_file(),
            "missing task-6 surface: {path}"
        );
    }

    let application = fs::read_to_string("src/application/groups/mod.rs")?;
    let repository = fs::read_to_string("src/adapters/postgres/groups/mod.rs")?
        + &fs::read_to_string("src/adapters/postgres/groups/mutation.rs")?;
    let transport = fs::read_to_string("src/transport/http/groups/mod.rs")?;
    assert!(!application.contains("sqlx::"));
    assert!(!application.contains("crate::adapters"));
    assert!(!application.contains("crate::transport"));
    assert!(!repository.contains(".begin()"));
    assert!(!repository.contains(".commit()"));
    assert!(!repository.contains(".rollback()"));
    assert!(!repository.contains("UnitOfWork"));
    for route in [
        "/api/v1/groups",
        "/api/v1/groups/{group_id}",
        "/api/v1/groups/{group_id}/members",
        "/api/v1/groups/{group_id}/members/{user_id}",
        "/api/v1/groups/{group_id}/invites",
        "/api/v1/invites/{code}/join",
    ] {
        assert!(transport.contains(route), "missing task-6 route: {route}");
    }
    Ok(())
}
