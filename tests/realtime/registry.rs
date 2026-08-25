use jamye_server::transport::realtime::LocalRealtimeHub;
use uuid::Uuid;

#[tokio::test]
async fn cleanup_atomically_removes_socket_user_and_conversation_entries() {
    let hub = LocalRealtimeHub::default();
    let conversation_id = Uuid::new_v4();
    let mut connection = hub.register(Uuid::new_v4()).await;
    assert!(hub.subscribe(connection.socket_id, conversation_id).await);
    assert_eq!(hub.registry_counts().await, (1, 1, 1));
    assert_eq!(hub.publish(conversation_id, "first".to_owned()).await, 1);
    assert_eq!(connection.outbound.recv().await.as_deref(), Some("first"));

    assert!(hub.cleanup(connection.socket_id).await);
    assert_eq!(hub.registry_counts().await, (0, 0, 0));
    assert_eq!(hub.publish(conversation_id, "late".to_owned()).await, 0);
    assert_eq!(connection.outbound.recv().await, None);
}
