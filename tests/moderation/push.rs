//! Operator report alert: enqueue after commit through the push delivery table and delivery by
//! the existing push worker.

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use axum::http::StatusCode;
use jamye_server::{
    adapters::postgres::{push::PostgresPushRepository, transactions::SqlxTransactionManager},
    application::push::{PushWorker, PushWorkerConfig, PushWorkerDependencies},
    ports::push::{
        PushProvider, PushProviderError, PushProviderFuture, PushProviderOutcome,
        PushProviderRequest, PushReportAlertRequest,
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    TestResult,
    support::{
        Harness, insert_group, insert_installation, insert_message, insert_user,
        insert_user_with_id, json_body, report_message_body, report_user_body,
    },
};

const MESSAGE_TEXT: &str = "private message text that must not reach the alert";

struct Scenario {
    reporter: Uuid,
    target: Uuid,
    message_id: Uuid,
}

async fn scenario(harness: &Harness) -> TestResult<Scenario> {
    let reporter = insert_user(&harness.pool, "reporter").await?;
    let target = insert_user(&harness.pool, "target").await?;
    let group = insert_group(&harness.pool, &[reporter, target]).await?;
    let message_id = insert_message(&harness.pool, group.chatroom_id, target, MESSAGE_TEXT).await?;
    Ok(Scenario {
        reporter,
        target,
        message_id,
    })
}

async fn report_id_of(response: axum::http::Response<axum::body::Body>) -> TestResult<Uuid> {
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = json_body(response).await?;
    Ok(Uuid::try_parse(body["id"].as_str().ok_or("id missing")?)?)
}

#[tokio::test]
async fn a_committed_report_enqueues_one_generic_alert_per_operator_installation() -> TestResult {
    let operator_a = Uuid::new_v4();
    let operator_b = Uuid::new_v4();
    let harness = Harness::with_operators(vec![operator_a, operator_b]).await?;
    let result: TestResult = async {
        insert_user_with_id(&harness.pool, operator_a, "operator a").await?;
        insert_user_with_id(&harness.pool, operator_b, "operator b").await?;
        let scenario = scenario(&harness).await?;
        let installation_a = insert_installation(&harness.pool, operator_a).await?;
        let installation_b = insert_installation(&harness.pool, operator_b).await?;
        insert_installation(&harness.pool, scenario.reporter).await?;
        insert_installation(&harness.pool, scenario.target).await?;

        let report_id = report_id_of(
            harness
                .report(
                    scenario.reporter,
                    &report_message_body(scenario.message_id, "hate"),
                )
                .await?,
        )
        .await?;

        let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid, bool, bool, String, Value, bool)>(
            "SELECT report_id, recipient_user_id, push_installation_id, \
                    notification_id IS NULL, source_event_id IS NULL, status, payload, \
                    message_preview_enabled_snapshot \
             FROM push_delivery_intents ORDER BY recipient_user_id",
        )
        .fetch_all(&harness.pool)
        .await?;
        assert_eq!(
            rows.len(),
            2,
            "one alert per operator installation, nobody else"
        );
        let mut installations = Vec::new();
        for row in &rows {
            assert_eq!(row.0, report_id);
            assert!([operator_a, operator_b].contains(&row.1));
            assert!(
                row.3 && row.4,
                "an alert has no notification or conversation event"
            );
            assert_eq!(row.5, "pending");
            assert_eq!(
                row.6,
                json!({"type": "report", "report_id": report_id.to_string()})
            );
            assert!(!row.7);
            assert!(!row.6.to_string().contains(MESSAGE_TEXT));
            installations.push(row.2);
        }
        installations.sort();
        let mut expected = vec![installation_a, installation_b];
        expected.sort();
        assert_eq!(installations, expected);

        // The alert is created once per report: a second report adds its own alerts only.
        let second = report_id_of(
            harness
                .report(
                    scenario.reporter,
                    &report_user_body(scenario.target, "spam"),
                )
                .await?,
        )
        .await?;
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM push_delivery_intents WHERE report_id = $1",
        )
        .bind(second)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(total, 2);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn without_operator_accounts_no_alert_is_sent() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let scenario = scenario(&harness).await?;
        insert_installation(&harness.pool, scenario.target).await?;
        report_id_of(
            harness
                .report(
                    scenario.reporter,
                    &report_user_body(scenario.target, "spam"),
                )
                .await?,
        )
        .await?;
        let rows = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM push_delivery_intents")
            .fetch_one(&harness.pool)
            .await?;
        assert_eq!(rows, 0);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn disabled_installations_and_deleted_operators_get_no_alert() -> TestResult {
    let operator = Uuid::new_v4();
    let gone = Uuid::new_v4();
    let harness = Harness::with_operators(vec![operator, gone]).await?;
    let result: TestResult = async {
        insert_user_with_id(&harness.pool, operator, "operator").await?;
        insert_user_with_id(&harness.pool, gone, "gone operator").await?;
        let scenario = scenario(&harness).await?;
        let disabled = insert_installation(&harness.pool, operator).await?;
        sqlx::query("UPDATE push_installations SET disabled_at = clock_timestamp() WHERE id = $1")
            .bind(disabled)
            .execute(&harness.pool)
            .await?;
        insert_installation(&harness.pool, gone).await?;
        sqlx::query("UPDATE users SET deleted_at = clock_timestamp() WHERE id = $1")
            .bind(gone)
            .execute(&harness.pool)
            .await?;
        report_id_of(
            harness
                .report(
                    scenario.reporter,
                    &report_user_body(scenario.target, "spam"),
                )
                .await?,
        )
        .await?;
        let rows = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM push_delivery_intents")
            .fetch_one(&harness.pool)
            .await?;
        assert_eq!(rows, 0);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn an_enqueue_failure_never_fails_the_report() -> TestResult {
    let operator = Uuid::new_v4();
    let harness = Harness::with_operators(vec![operator]).await?;
    let result: TestResult = async {
        insert_user_with_id(&harness.pool, operator, "operator").await?;
        let scenario = scenario(&harness).await?;
        insert_installation(&harness.pool, operator).await?;
        // Break only the alert path: the report tables stay intact.
        sqlx::query("ALTER TABLE push_delivery_intents RENAME TO push_delivery_intents_broken")
            .execute(&harness.pool)
            .await?;
        let report_id = report_id_of(
            harness
                .report(
                    scenario.reporter,
                    &report_user_body(scenario.target, "spam"),
                )
                .await?,
        )
        .await?;
        let stored = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM reports WHERE id = $1")
            .bind(report_id)
            .fetch_one(&harness.pool)
            .await?;
        assert_eq!(stored, 1, "the report must stay stored");
        Ok(())
    }
    .await;
    harness.finish(result).await
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => panic!("alert provider mutex is poisoned"),
    }
}

struct AlertProvider {
    alerts: Mutex<Vec<PushReportAlertRequest>>,
    outcome: PushProviderOutcome,
}

impl PushProvider for AlertProvider {
    fn send<'a>(&'a self, _request: &'a PushProviderRequest) -> PushProviderFuture<'a> {
        Box::pin(async { Err(PushProviderError::Rejected) })
    }

    fn send_report_alert<'a>(
        &'a self,
        request: &'a PushReportAlertRequest,
    ) -> PushProviderFuture<'a> {
        Box::pin(async move {
            lock(&self.alerts).push(request.clone());
            Ok(self.outcome)
        })
    }
}

fn worker(pool: &PgPool, provider: Arc<AlertProvider>) -> TestResult<PushWorker> {
    let repository = Arc::new(PostgresPushRepository::new(pool.clone()));
    Ok(PushWorker::new(
        PushWorkerDependencies {
            transactions: Arc::new(SqlxTransactionManager::new(pool.clone())),
            repository: repository.clone(),
            preview_source: repository,
            provider,
        },
        PushWorkerConfig {
            claim_owner: "moderation-test-worker".to_owned(),
            batch_size: 10,
            lease_duration: Duration::from_secs(30),
            provider_timeout: Duration::from_secs(5),
            lease_safety_margin: Duration::from_secs(1),
            retry_delay: Duration::from_secs(1),
            poll_interval: Duration::from_millis(10),
            max_attempts: 3,
        },
    )?)
}

#[tokio::test]
async fn the_push_worker_delivers_the_alert_without_any_message_content() -> TestResult {
    let operator = Uuid::new_v4();
    let harness = Harness::with_operators(vec![operator]).await?;
    let result: TestResult = async {
        insert_user_with_id(&harness.pool, operator, "operator").await?;
        let scenario = scenario(&harness).await?;
        let installation = insert_installation(&harness.pool, operator).await?;
        let report_id = report_id_of(
            harness
                .report(
                    scenario.reporter,
                    &report_message_body(scenario.message_id, "illegal"),
                )
                .await?,
        )
        .await?;

        let provider = Arc::new(AlertProvider {
            alerts: Mutex::new(Vec::new()),
            outcome: PushProviderOutcome::Accepted,
        });
        let report = worker(&harness.pool, provider.clone())?.run_once().await?;
        assert_eq!(report.claimed, 1);
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.authorization_denied, 0);
        {
            let alerts = lock(&provider.alerts);
            assert_eq!(alerts.len(), 1);
            assert_eq!(alerts[0].report_id, report_id);
            assert_eq!(
                alerts[0].destination.token(),
                format!("ExponentPushToken[{installation}]")
            );
            assert!(!format!("{:?}", alerts[0]).contains(MESSAGE_TEXT));
        }
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM push_delivery_intents WHERE report_id = $1",
        )
        .bind(report_id)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(status, "succeeded");
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn an_unregistered_operator_device_is_disabled_like_any_other_push_destination() -> TestResult
{
    let operator = Uuid::new_v4();
    let harness = Harness::with_operators(vec![operator]).await?;
    let result: TestResult = async {
        insert_user_with_id(&harness.pool, operator, "operator").await?;
        let scenario = scenario(&harness).await?;
        let installation = insert_installation(&harness.pool, operator).await?;
        report_id_of(
            harness
                .report(
                    scenario.reporter,
                    &report_user_body(scenario.target, "spam"),
                )
                .await?,
        )
        .await?;
        let provider = Arc::new(AlertProvider {
            alerts: Mutex::new(Vec::new()),
            outcome: PushProviderOutcome::DeviceNotRegistered,
        });
        let report = worker(&harness.pool, provider)?.run_once().await?;
        assert_eq!(report.invalid_destinations, 1);
        let disabled = sqlx::query_scalar::<_, bool>(
            "SELECT disabled_at IS NOT NULL FROM push_installations WHERE id = $1",
        )
        .bind(installation)
        .fetch_one(&harness.pool)
        .await?;
        assert!(disabled);
        Ok(())
    }
    .await;
    harness.finish(result).await
}
