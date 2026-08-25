use std::{error::Error, fs, io};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const REALTIME_APPLICATION_ROOT: &str = "src/application/realtime/mod.rs";

#[test]
fn realtime_surface_is_absent_before_task_4b() -> TestResult {
    fs::read_to_string(REALTIME_APPLICATION_ROOT)
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::other(format!(
                    "RED: {REALTIME_APPLICATION_ROOT} is absent; task-4b must add outbox delivery, Redis tickets/PubSub, WebSocket transport, and static C1 composition"
                ))
                .into()
            } else {
                error.into()
            }
        })
}
