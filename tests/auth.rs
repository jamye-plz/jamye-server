use std::{error::Error, fs, io};

#[path = "auth/helpers.rs"]
mod auth_helpers;
#[path = "auth/contract.rs"]
mod contract;
#[path = "auth/google_id_token.rs"]
mod google_id_token;
#[path = "auth/migration.rs"]
mod migration;
#[path = "auth/oauth.rs"]
mod oauth;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "auth/session.rs"]
mod session;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[test]
fn production_auth_surface_is_statically_registered() -> io::Result<()> {
    for path in [
        "migrations/0002_auth_sessions.sql",
        "src/application/auth/service.rs",
        "src/application/users/mod.rs",
        "src/ports/auth/mod.rs",
        "src/ports/oauth_attempt/mod.rs",
        "src/ports/oauth_provider/mod.rs",
        "src/ports/rate_limit/mod.rs",
        "src/adapters/oauth/providers.rs",
        "src/adapters/postgres/auth/mod.rs",
        "src/adapters/redis/oauth_attempt/mod.rs",
        "src/adapters/redis/rate_limit/mod.rs",
        "src/transport/http/auth/api.rs",
        "src/transport/http/users/mod.rs",
        "contracts/contributions/task-5/dto/operations.json",
        "contracts/contributions/task-5/schemas/auth-wire.schema.json",
        "contracts/contributions/task-5/fixtures/mobile-auth-handoff.json",
        "docs/adr/0005-rate-limit-coordination.md",
        "docs/adr/0006-mobile-oauth.md",
    ] {
        assert!(
            fs::metadata(path)?.is_file(),
            "missing task-5 surface: {path}"
        );
    }

    let application = fs::read_to_string("src/application/auth/service.rs")?
        + &fs::read_to_string("src/application/users/mod.rs")?;
    let repository = fs::read_to_string("src/adapters/postgres/auth/mod.rs")?;
    let provider = fs::read_to_string("src/adapters/oauth/providers.rs")?;
    assert!(!application.contains("sqlx::"));
    assert!(!application.contains("crate::adapters"));
    assert!(!application.contains("crate::transport"));
    assert!(!repository.contains(".begin()"));
    assert!(!repository.contains(".commit()"));
    assert!(!repository.contains(".rollback()"));
    assert!(!provider.contains("token_url:"));
    assert!(!provider.contains("identity_url:"));
    assert!(!provider.contains("jwks_url:"));
    assert!(provider.contains(".redirect(Policy::none())"));
    assert!(!provider.contains("danger_accept_invalid_certs"));
    Ok(())
}
