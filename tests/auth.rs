use std::{error::Error, fs, io};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const AUTH_MIGRATION: &str = "migrations/0002_auth_sessions.sql";

#[test]
fn production_auth_surface_is_absent_before_task_5() -> TestResult {
    fs::read_to_string(AUTH_MIGRATION)
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::other(format!(
                    "RED: {AUTH_MIGRATION} is absent; task-5 must add D12=A PKCE OAuth, hashed refresh families, shared rate limiting, and profile surfaces"
                ))
                .into()
            } else {
                error.into()
            }
        })
}
