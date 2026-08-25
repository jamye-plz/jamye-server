use std::{error::Error, fs, io};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const MESSAGING_DOMAIN_ROOT: &str = "src/domain/messaging/mod.rs";

#[test]
fn reliable_messaging_surface_is_absent_before_task_4a() -> TestResult {
    fs::read_to_string(MESSAGING_DOMAIN_ROOT).map(|_| ()).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {MESSAGING_DOMAIN_ROOT} is absent; task-4a must add the reliable messaging domain, transaction, PostgreSQL, and HTTP surfaces"
            ))
            .into()
        } else {
            error.into()
        }
    })
}
