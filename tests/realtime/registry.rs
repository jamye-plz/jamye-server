use std::sync::Arc;

use jamye_server::transport::realtime::LocalRealtimeHub;
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::TestResult;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_admission_never_exceeds_eight() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let user_id = Uuid::new_v4();
    let barrier = Arc::new(Barrier::new(64));
    let mut tasks = Vec::new();
    for _ in 0..64 {
        let hub = hub.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            hub.register(user_id).await
        }));
    }
    let mut admitted = Vec::new();
    let mut rejected = 0;
    for task in tasks {
        match task.await? {
            Ok(connection) => admitted.push(connection),
            Err(_) => rejected += 1,
        }
    }
    assert_eq!(admitted.len(), 8);
    assert_eq!(rejected, 56);
    assert_eq!(hub.registry_counts().await, (8, 8, 0));

    let other_user = hub.register(Uuid::new_v4()).await?;
    assert_eq!(hub.registry_counts().await, (9, 9, 0));
    assert!(hub.cleanup(admitted[0].socket_id).await);
    let replacement = hub.register(user_id).await?;
    assert_eq!(hub.registry_counts().await, (9, 9, 0));
    assert!(hub.cleanup(replacement.socket_id).await);
    assert!(hub.cleanup(other_user.socket_id).await);
    for connection in admitted.iter().skip(1) {
        assert!(hub.cleanup(connection.socket_id).await);
    }
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    Ok(())
}

#[tokio::test]
async fn cleanup_atomically_removes_socket_user_and_conversation_entries() -> TestResult {
    let hub = LocalRealtimeHub::default();
    let conversation_id = Uuid::new_v4();
    let mut connection = hub.register(Uuid::new_v4()).await?;
    assert!(hub.subscribe(connection.socket_id, conversation_id).await);
    assert_eq!(hub.registry_counts().await, (1, 1, 1));
    assert_eq!(hub.publish(conversation_id, "first".to_owned()).await, 1);
    assert_eq!(connection.outbound.recv().await.as_deref(), Some("first"));

    assert!(hub.cleanup(connection.socket_id).await);
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    assert_eq!(hub.publish(conversation_id, "late".to_owned()).await, 0);
    assert_eq!(connection.outbound.recv().await, None);
    Ok(())
}
