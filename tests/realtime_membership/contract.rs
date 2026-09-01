use std::fs;

use jamye_server::transport::realtime::{CLOSE_MEMBERSHIP_REQUIRED, RealtimeEvictionReason};
use serde_json::Value;

use crate::TestResult;

#[test]
fn internal_control_types_never_enter_the_public_realtime_event_union() -> TestResult {
    assert_eq!(CLOSE_MEMBERSHIP_REQUIRED, 4001);
    assert_eq!(
        RealtimeEvictionReason::MembershipRevoked.as_str(),
        "membership_revoked"
    );
    assert_eq!(
        RealtimeEvictionReason::GroupDeleted.as_str(),
        "group_deleted"
    );

    let manifest: Value = serde_json::from_str(&fs::read_to_string("contracts/manifest.json")?)?;
    assert_eq!(
        manifest["realtime_discriminants"],
        serde_json::json!(["message.created", "topic.created"])
    );
    for path in [
        "contracts/manifest.json",
        "contracts/realtime/server-frame.schema.json",
        "contracts/realtime/protocol.json",
    ] {
        let public_contract = fs::read_to_string(path)?;
        assert!(!public_contract.contains("membership.revoked"));
        assert!(!public_contract.contains("group.deleted"));
        assert!(!public_contract.contains("control_id"));
    }
    Ok(())
}
