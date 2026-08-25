use std::{fs, io};

#[cfg(feature = "dev-fixtures")]
#[path = "messaging/delta.rs"]
mod delta;
#[cfg(feature = "dev-fixtures")]
#[path = "messaging/helpers.rs"]
mod messaging_helpers;
#[cfg(feature = "dev-fixtures")]
#[path = "messaging/http.rs"]
mod messaging_http;
#[cfg(feature = "dev-fixtures")]
#[path = "support/postgres.rs"]
mod postgres_support;
#[cfg(feature = "dev-fixtures")]
#[path = "messaging/recovery.rs"]
mod recovery;
#[path = "messaging/transaction.rs"]
mod transaction;

#[test]
fn reliable_messaging_surface_is_registered_without_a_parallel_uow() -> io::Result<()> {
    for path in [
        "src/domain/messaging/mod.rs",
        "src/application/messaging/mod.rs",
        "src/ports/messaging/mod.rs",
        "src/ports/transactions/mod.rs",
        "src/adapters/postgres/messaging/mod.rs",
        "src/adapters/postgres/transactions/mod.rs",
        "src/transport/http/messaging/mod.rs",
    ] {
        assert!(
            fs::metadata(path)?.is_file(),
            "missing task-4a surface: {path}"
        );
    }

    let application = fs::read_to_string("src/application/messaging/mod.rs")?;
    let repository = fs::read_to_string("src/adapters/postgres/messaging/send.rs")?;
    let delta = fs::read_to_string("src/adapters/postgres/messaging/delta.rs")?;
    let transport = fs::read_to_string("src/transport/http/messaging/mod.rs")?;
    let transaction_port = fs::read_to_string("src/ports/transactions/mod.rs")?;
    assert!(!application.contains("sqlx::"));
    assert!(!repository.contains(".begin()"));
    assert!(!repository.contains(".commit()"));
    assert!(!repository.contains(".rollback()"));
    assert!(!transaction_port.contains("UnitOfWork"));
    let messaging_source = [application, repository, delta, transport].join("\n");
    assert!(!messaging_source.contains("redis"));
    assert!(!messaging_source.contains("WebSocket"));
    assert!(!messaging_source.contains("websocket"));
    Ok(())
}
