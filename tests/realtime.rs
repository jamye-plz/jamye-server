use std::{fs, io};

#[path = "realtime/outbox.rs"]
mod outbox;
#[path = "realtime/recovery.rs"]
mod recovery;
#[path = "realtime/redis.rs"]
mod redis;
#[path = "realtime/redis_recovery.rs"]
mod redis_recovery;
#[path = "realtime/registry.rs"]
mod registry;
#[path = "realtime/ticket.rs"]
mod ticket;
#[path = "realtime/websocket.rs"]
mod websocket;

#[cfg(feature = "dev-fixtures")]
#[path = "realtime/c1.rs"]
mod c1;

#[path = "support/postgres.rs"]
mod postgres_support;
pub use postgres_support::TestResult;
#[path = "support/fixtures.rs"]
mod fixture_support;

#[test]
fn realtime_surface_is_statically_registered_without_a_parallel_uow() -> io::Result<()> {
    for path in [
        "src/application/realtime/mod.rs",
        "src/ports/realtime/mod.rs",
        "src/adapters/postgres/realtime/mod.rs",
        "src/adapters/redis/realtime/mod.rs",
        "src/transport/http/realtime/mod.rs",
        "src/transport/realtime/mod.rs",
    ] {
        assert!(
            fs::metadata(path)?.is_file(),
            "missing task-4b surface: {path}"
        );
    }

    let application = fs::read_to_string("src/application/realtime/mod.rs")?
        + &fs::read_to_string("src/application/realtime/outbox.rs")?
        + &fs::read_to_string("src/application/realtime/ticket.rs")?;
    let postgres = fs::read_to_string("src/adapters/postgres/realtime/mod.rs")?;
    let registry = fs::read_to_string("src/transport/realtime/registry.rs")?;
    assert!(!application.contains("crate::adapters"));
    assert!(!application.contains("crate::transport"));
    assert!(postgres.contains("SKIP LOCKED"));
    assert!(postgres.contains("clock_timestamp()"));
    assert!(postgres.contains("claim_generation = $3"));
    assert!(!registry.contains("inventory::"));
    assert!(!registry.contains("register_handler"));
    Ok(())
}
