use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use jamye_server::{
    application::push::{
        ExpoInstallationCreateInput, ExpoInstallationPutInput, PushDependencies, PushError,
        PushInstallation, PushInstallationUpsert, PushService,
    },
    ports::{
        push::{
            DeletePushInstallationCommand, PushEnvironment, PushInstallationRecord, PushPlatform,
            PushProviderName, PushRepository, PushRepositoryError, PushRepositoryFuture,
            UpdatePushInstallationCommand, UpsertPushInstallationCommand,
            UpsertPushInstallationOutcome,
        },
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

const INSTALLATION_ID: &str = "ios-device-stable-id";
const TOKEN: &str = "ExponentPushToken[task-9-current]";

#[tokio::test]
async fn p2_defaults_preview_false_and_sends_one_server_fixed_expo_upsert() {
    let harness = Harness::success();

    assert_eq!(
        harness
            .service
            .upsert_installation(user_id(), create_input(None))
            .await,
        Ok(PushInstallationUpsert {
            installation: public_installation(false),
            created: true,
        })
    );
    assert_eq!(
        harness.calls(),
        vec![Call::Begin, Call::Upsert, Call::Commit]
    );
    let commands = harness.repository.upsert_commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].user_id, user_id());
    assert_eq!(commands[0].installation_id, INSTALLATION_ID);
    assert_eq!(commands[0].platform, PushPlatform::Ios);
    assert_eq!(commands[0].environment, PushEnvironment::Development);
    assert_eq!(commands[0].provider, PushProviderName::Expo);
    assert_eq!(commands[0].token, TOKEN);
    assert!(!commands[0].message_preview_enabled);
}

#[tokio::test]
async fn p2_rejects_invalid_platform_environment_identity_and_token_without_side_effects() {
    let mut invalid = Vec::new();
    let mut platform = create_input(None);
    platform.platform = "watchos".to_owned();
    invalid.push(platform);
    let mut environment = create_input(None);
    environment.environment = "staging".to_owned();
    invalid.push(environment);
    let mut installation = create_input(None);
    installation.installation_id.clear();
    invalid.push(installation);
    let mut token = create_input(None);
    token.expo_token.clear();
    invalid.push(token);

    for input in invalid {
        let harness = Harness::success();
        assert_eq!(
            harness.service.upsert_installation(user_id(), input).await,
            Err(PushError::RequestValidation)
        );
        assert_eq!(harness.calls(), Vec::<Call>::new());
    }
}

#[tokio::test]
async fn p3_omission_preserves_preview_and_p4_targets_only_the_current_owner_installation() {
    let harness = Harness::success();

    assert_eq!(
        harness
            .service
            .update_installation(
                user_id(),
                INSTALLATION_ID.to_owned(),
                ExpoInstallationPutInput {
                    expo_token: TOKEN.to_owned(),
                    message_preview_enabled: None,
                },
            )
            .await,
        Ok(public_installation(false))
    );
    assert_eq!(
        harness
            .service
            .delete_installation(user_id(), INSTALLATION_ID.to_owned())
            .await,
        Ok(())
    );
    assert_eq!(
        harness.calls(),
        vec![
            Call::Begin,
            Call::Update,
            Call::Commit,
            Call::Begin,
            Call::Delete,
            Call::Commit,
        ]
    );
    assert_eq!(
        harness.repository.update_commands(),
        vec![UpdatePushInstallationCommand {
            user_id: user_id(),
            installation_id: INSTALLATION_ID.to_owned(),
            token: TOKEN.to_owned(),
            message_preview_enabled: None,
        }]
    );
    assert_eq!(
        harness.repository.delete_commands(),
        vec![DeletePushInstallationCommand {
            user_id: user_id(),
            installation_id: INSTALLATION_ID.to_owned(),
        }]
    );
}

#[tokio::test]
async fn p3_and_p4_stale_owner_paths_share_one_safe_not_found_and_rollback() {
    let update = Harness::new(
        Ok(upsert_outcome()),
        Err(PushRepositoryError::InstallationNotFound),
        Ok(()),
    );
    assert_eq!(
        update
            .service
            .update_installation(
                stale_user_id(),
                INSTALLATION_ID.to_owned(),
                ExpoInstallationPutInput {
                    expo_token: TOKEN.to_owned(),
                    message_preview_enabled: Some(true),
                },
            )
            .await,
        Err(PushError::InstallationNotFound)
    );
    assert_eq!(
        update.calls(),
        vec![Call::Begin, Call::Update, Call::Rollback]
    );

    let delete = Harness::new(
        Ok(upsert_outcome()),
        Ok(installation_record(false)),
        Err(PushRepositoryError::InstallationNotFound),
    );
    assert_eq!(
        delete
            .service
            .delete_installation(stale_user_id(), INSTALLATION_ID.to_owned())
            .await,
        Err(PushError::InstallationNotFound)
    );
    assert_eq!(
        delete.calls(),
        vec![Call::Begin, Call::Delete, Call::Rollback]
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Begin,
    Upsert,
    Update,
    Delete,
    Commit,
    Rollback,
}

struct Harness {
    service: PushService,
    calls: Arc<Mutex<Vec<Call>>>,
    repository: Arc<RecordingRepository>,
}

impl Harness {
    fn success() -> Self {
        Self::new(Ok(upsert_outcome()), Ok(installation_record(false)), Ok(()))
    }

    fn new(
        upsert_result: Result<UpsertPushInstallationOutcome, PushRepositoryError>,
        update_result: Result<PushInstallationRecord, PushRepositoryError>,
        delete_result: Result<(), PushRepositoryError>,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transactions = Arc::new(RecordingTransactions {
            calls: calls.clone(),
        });
        let repository = Arc::new(RecordingRepository {
            calls: calls.clone(),
            upsert_commands: Mutex::new(Vec::new()),
            update_commands: Mutex::new(Vec::new()),
            delete_commands: Mutex::new(Vec::new()),
            upsert_result,
            update_result,
            delete_result,
        });
        let service = PushService::new(PushDependencies {
            transactions,
            repository: repository.clone(),
        });
        Self {
            service,
            calls,
            repository,
        }
    }

    fn calls(&self) -> Vec<Call> {
        crate::lock_test_mutex(&self.calls, "call").clone()
    }
}

struct RecordingHandle;

impl TransactionHandle for RecordingHandle {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

struct RecordingTransactions {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl TransactionManager for RecordingTransactions {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        record(&self.calls, Call::Begin);
        Box::pin(async { Ok(Box::new(RecordingHandle) as BoxTransactionHandle) })
    }

    fn commit<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        record(&self.calls, Call::Commit);
        Box::pin(async { Ok(()) })
    }

    fn rollback<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        record(&self.calls, Call::Rollback);
        Box::pin(async { Ok(()) })
    }
}

struct RecordingRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    upsert_commands: Mutex<Vec<UpsertPushInstallationCommand>>,
    update_commands: Mutex<Vec<UpdatePushInstallationCommand>>,
    delete_commands: Mutex<Vec<DeletePushInstallationCommand>>,
    upsert_result: Result<UpsertPushInstallationOutcome, PushRepositoryError>,
    update_result: Result<PushInstallationRecord, PushRepositoryError>,
    delete_result: Result<(), PushRepositoryError>,
}

impl RecordingRepository {
    fn upsert_commands(&self) -> Vec<UpsertPushInstallationCommand> {
        crate::lock_test_mutex(&self.upsert_commands, "upsert command").clone()
    }

    fn update_commands(&self) -> Vec<UpdatePushInstallationCommand> {
        crate::lock_test_mutex(&self.update_commands, "update command").clone()
    }

    fn delete_commands(&self) -> Vec<DeletePushInstallationCommand> {
        crate::lock_test_mutex(&self.delete_commands, "delete command").clone()
    }
}

impl PushRepository for RecordingRepository {
    fn upsert_installation<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        command: &'a UpsertPushInstallationCommand,
    ) -> PushRepositoryFuture<'a, UpsertPushInstallationOutcome> {
        record(&self.calls, Call::Upsert);
        crate::lock_test_mutex(&self.upsert_commands, "upsert command").push(command.clone());
        let result = self.upsert_result.clone();
        Box::pin(async move { result })
    }

    fn update_installation<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        command: &'a UpdatePushInstallationCommand,
    ) -> PushRepositoryFuture<'a, PushInstallationRecord> {
        record(&self.calls, Call::Update);
        crate::lock_test_mutex(&self.update_commands, "update command").push(command.clone());
        let result = self.update_result.clone();
        Box::pin(async move { result })
    }

    fn delete_installation<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        command: &'a DeletePushInstallationCommand,
    ) -> PushRepositoryFuture<'a, ()> {
        record(&self.calls, Call::Delete);
        crate::lock_test_mutex(&self.delete_commands, "delete command").push(command.clone());
        let result = self.delete_result;
        Box::pin(async move { result })
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    crate::lock_test_mutex(calls, "call").push(call);
}

fn create_input(message_preview_enabled: Option<bool>) -> ExpoInstallationCreateInput {
    ExpoInstallationCreateInput {
        platform: "ios".to_owned(),
        environment: "development".to_owned(),
        installation_id: INSTALLATION_ID.to_owned(),
        expo_token: TOKEN.to_owned(),
        message_preview_enabled,
    }
}

fn upsert_outcome() -> UpsertPushInstallationOutcome {
    UpsertPushInstallationOutcome {
        installation: installation_record(false),
        created: true,
    }
}

fn installation_record(message_preview_enabled: bool) -> PushInstallationRecord {
    PushInstallationRecord {
        id: Uuid::from_u128(0x11111111_1111_4111_8111_111111111111),
        user_id: user_id(),
        owner_epoch: 1,
        installation_id: INSTALLATION_ID.to_owned(),
        platform: PushPlatform::Ios,
        provider: PushProviderName::Expo,
        token: TOKEN.to_owned(),
        environment: PushEnvironment::Development,
        message_preview_enabled,
        last_seen_at: OffsetDateTime::UNIX_EPOCH,
        disabled_at: None,
    }
}

fn public_installation(message_preview_enabled: bool) -> PushInstallation {
    PushInstallation {
        installation_id: INSTALLATION_ID.to_owned(),
        platform: PushPlatform::Ios,
        environment: PushEnvironment::Development,
        provider: PushProviderName::Expo,
        message_preview_enabled,
        last_seen_at: OffsetDateTime::UNIX_EPOCH,
        disabled_at: None,
    }
}

fn user_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn stale_user_id() -> Uuid {
    Uuid::from_u128(0xbbbbbbbb_bbbb_4bbb_8bbb_bbbbbbbbbbbb)
}
