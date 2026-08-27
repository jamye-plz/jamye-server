use std::{error::Error, fs, io};

#[path = "realtime_membership/contract.rs"]
mod contract;
#[path = "realtime_membership/control.rs"]
mod control;
#[path = "realtime_membership/delivery.rs"]
mod delivery;
#[path = "realtime_membership/helpers.rs"]
mod helpers;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "realtime_membership/transaction.rs"]
mod transaction;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[test]
fn production_realtime_membership_surface_is_statically_registered() -> io::Result<()> {
    for path in [
        "src/application/realtime/membership_revocation/mod.rs",
        "src/adapters/postgres/realtime_revocations/mod.rs",
        "src/adapters/redis/realtime_control/mod.rs",
        "src/transport/realtime/authorization/mod.rs",
        "src/transport/realtime/registry/revocation.rs",
        "docs/commands/task-6c/realtime-membership.md",
        "scripts/tasks/task-6c/mod.just",
    ] {
        assert!(
            fs::metadata(path)?.is_file(),
            "missing task-6c surface: {path}"
        );
    }

    let application = fs::read_to_string("src/application/realtime/membership_revocation/mod.rs")?;
    let groups = fs::read_to_string("src/application/groups/mod.rs")?;
    let postgres = fs::read_to_string("src/adapters/postgres/realtime_revocations/mod.rs")?;
    assert!(!application.contains("crate::adapters"));
    assert!(!application.contains("crate::transport"));
    assert!(groups.contains("remove_member_in_transaction"));
    assert!(groups.contains("delete_group_in_transaction"));
    assert!(postgres.contains("intent_type = 'control'"));
    assert!(postgres.contains("SKIP LOCKED"));
    assert!(postgres.contains("ANY($2::UUID[])"));
    Ok(())
}
