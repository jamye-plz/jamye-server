//! Composition and execution of admin commands. The composition root connects to PostgreSQL
//! only (no Redis, object storage or listener) and wires the existing application services.

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{
            account_deletion::PostgresAccountDeletionRepository, groups::PostgresGroupsRepository,
            messaging::PostgresMessagingRepository, moderation::PostgresModerationRepository,
            push::PostgresPushRepository, runtime_pool, transactions::SqlxTransactionManager,
        },
    },
    application::{
        account_deletion::{
            AccountDeletionDependencies, AccountDeletionError, AccountDeletionService,
        },
        groups::{GroupsDependencies, GroupsService, SystemGroupsClock},
        moderation::{
            ModerationDependencies, ModerationError, ModerationService, ModerationSettings,
            admin::{
                ModerationAdminDependencies, ModerationAdminError, ModerationAdminService,
                ReportChange,
            },
        },
    },
    config::{AppConfig, rate_limit::RateLimitConfig},
    ports::rate_limit::{RateLimitError, RateLimitFuture, RateLimitRequest, RateLimiter},
    transport::admin::{
        cli::Command,
        render::{report_json, report_list_text, report_text},
    },
};

/// Process exit codes of the admin binary.
pub const EXIT_FAILURE: u8 = 1;
pub const EXIT_USAGE: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdminCompositionError {
    Postgres,
    Groups,
}

impl std::fmt::Display for AdminCompositionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Postgres => "the PostgreSQL connection settings are invalid",
            Self::Groups => "the groups service could not be composed",
        })
    }
}

impl std::error::Error for AdminCompositionError {}

#[derive(Clone)]
pub struct AdminServices {
    pub moderation: Arc<ModerationService>,
    pub moderation_admin: Arc<ModerationAdminService>,
    pub accounts: Arc<AccountDeletionService>,
}

/// The admin CLI has no Redis. Report creation and group invites are the only rate-limited
/// paths and the CLI never reaches them, so this limiter fails closed if one ever does.
struct OfflineRateLimiter;

impl RateLimiter for OfflineRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Err(RateLimitError) })
    }
}

/// Builds the admin services from the same `AppConfig` as the API. The pool connects lazily,
/// so an unreachable database surfaces as a failed command, not as a startup panic.
pub fn compose(config: &AppConfig) -> Result<AdminServices, AdminCompositionError> {
    let pool = runtime_pool(config.database_url(), config.readiness_timeout())
        .map_err(|_| AdminCompositionError::Postgres)?;
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    let moderation_repository = Arc::new(PostgresModerationRepository::new(pool.clone()));
    let offline_limiter = Arc::new(OfflineRateLimiter);

    let moderation = Arc::new(ModerationService::new(
        ModerationDependencies {
            transactions: transactions.clone(),
            repository: moderation_repository.clone(),
            rate_limiter: offline_limiter.clone(),
        },
        ModerationSettings::default(),
    ));
    let moderation_admin = Arc::new(ModerationAdminService::new(ModerationAdminDependencies {
        transactions: transactions.clone(),
        repository: moderation_repository,
        messaging: Arc::new(PostgresMessagingRepository::new(pool.clone())),
    }));

    // The deletion service carries a groups service it does not use on the grace path.
    let groups = Arc::new(
        GroupsService::new(
            GroupsDependencies {
                transactions: transactions.clone(),
                repository: Arc::new(PostgresGroupsRepository::new(pool.clone())),
                rate_limiter: offline_limiter,
                credentials: Arc::new(OsCredentialSource),
                clock: Arc::new(SystemGroupsClock),
            },
            RateLimitConfig::default().groups,
        )
        .map_err(|_| AdminCompositionError::Groups)?,
    );
    let accounts = Arc::new(AccountDeletionService::new(AccountDeletionDependencies {
        transactions,
        groups,
        push_privacy_fence: Arc::new(PostgresPushRepository::new(pool.clone())),
        repository: Arc::new(PostgresAccountDeletionRepository::new(pool)),
        apple_identity_provider: None,
        apple_revocation_provider: None,
    }));
    Ok(AdminServices {
        moderation,
        moderation_admin,
        accounts,
    })
}

/// A failed command: `Usage` maps to exit status 2, `Failure` to exit status 1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandError {
    Usage(String),
    Failure(String),
}

impl CommandError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => EXIT_USAGE,
            Self::Failure(_) => EXIT_FAILURE,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Usage(message) | Self::Failure(message) => message,
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for CommandError {}

const DATABASE_FAILURE: &str = "the database operation failed; see the structured log lines";

/// Runs one command and returns the text for stdout. Mutations log one structured line in the
/// application services.
pub async fn execute(services: &AdminServices, command: Command) -> Result<String, CommandError> {
    match command {
        Command::ReportsList {
            status,
            limit,
            json,
        } => {
            let reports = services
                .moderation_admin
                .list_reports(status, limit)
                .await
                .map_err(|error| admin_failure(error, "reports list"))?;
            if json {
                let items = reports.iter().map(report_json).collect::<Vec<_>>();
                render_json(&serde_json::Value::Array(items))
            } else {
                Ok(report_list_text(&reports))
            }
        }
        Command::ReportsShow { report_id, json } => {
            let report = services
                .moderation_admin
                .show_report(report_id)
                .await
                .map_err(|error| report_failure(error, report_id, "shown"))?;
            if json {
                render_json(&report_json(&report))
            } else {
                Ok(report_text(&report))
            }
        }
        Command::ReportsDismiss { report_id } => {
            let change = services
                .moderation_admin
                .dismiss_report(report_id)
                .await
                .map_err(|error| report_failure(error, report_id, "dismissed"))?;
            Ok(match change {
                ReportChange::Changed => format!("report {report_id} dismissed\n"),
                ReportChange::Unchanged => {
                    format!("report {report_id} was already dismissed; no change\n")
                }
            })
        }
        Command::ReportsResolve { report_id } => {
            let change = services
                .moderation_admin
                .resolve_report(report_id)
                .await
                .map_err(|error| report_failure(error, report_id, "resolved"))?;
            Ok(match change {
                ReportChange::Changed => format!("report {report_id} resolved (actioned)\n"),
                ReportChange::Unchanged => {
                    format!("report {report_id} was already resolved (actioned); no change\n")
                }
            })
        }
        Command::ReportsPurge { older_than_days } => {
            let deleted = services
                .moderation_admin
                .purge_handled_reports(older_than_days)
                .await
                .map_err(|error| admin_failure(error, "reports purge"))?;
            Ok(format!(
                "purged {deleted} handled report(s) handled more than {older_than_days} day(s) ago\n"
            ))
        }
        Command::MessagesDelete { message_id } => {
            let deletion = services
                .moderation_admin
                .delete_message(message_id)
                .await
                .map_err(|error| match error {
                    ModerationAdminError::MessageNotFound => CommandError::Failure(format!(
                        "message {message_id} not found, or its group or topic no longer exists"
                    )),
                    ModerationAdminError::NotUserMessage => CommandError::Failure(format!(
                        "message {message_id} is not a user message and cannot be moderated"
                    )),
                    other => admin_failure(other, "messages delete"),
                })?;
            let state = if deletion.already_deleted {
                "was already deleted; nothing to delete"
            } else {
                "deleted"
            };
            let reports = deletion
                .actioned_reports
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "message {message_id} {state} (chatroom {}, group {}); {} open report(s) actioned{}\n",
                deletion.chatroom_id,
                deletion.group_id,
                deletion.actioned_reports.len(),
                if reports.is_empty() {
                    String::new()
                } else {
                    format!(": {reports}")
                }
            ))
        }
        Command::UsersSuspend { user_id, reason } => {
            services
                .moderation
                .suspend_user(user_id, reason)
                .await
                .map_err(|error| user_failure(error, user_id))?;
            Ok(format!(
                "user {user_id} suspended; refresh sessions revoked (an open WebSocket is not closed; it ends when its access token expires)\n"
            ))
        }
        Command::UsersUnsuspend { user_id } => {
            services
                .moderation
                .unsuspend_user(user_id)
                .await
                .map_err(|error| user_failure(error, user_id))?;
            Ok(format!(
                "user {user_id} unsuspended; the user must sign in again\n"
            ))
        }
        Command::AccountsDelete { user_id } => {
            let deletion = services
                .accounts
                .start_operator_grace_deletion(user_id)
                .await
                .map_err(|error| account_failure(error, user_id))?;
            let mut output = format!(
                "account {user_id} deletion started: the grace period begins now and the worker purges the account when it ends (30 days unless JAMYE_ACCOUNT_PURGE_GRACE_DAYS says otherwise); {} membership(s) removed\n",
                deletion.report.memberships_removed
            );
            if deletion.apple_authorization_not_revoked {
                output.push_str(
                    "warning: this account signs in with Apple. The Sign in with Apple authorization was NOT revoked (it needs the user's re-authentication); the user can revoke it in iOS Settings > Apple ID > Sign-In & Security > Sign in with Apple\n",
                );
            }
            Ok(output)
        }
    }
}

fn render_json(value: &serde_json::Value) -> Result<String, CommandError> {
    serde_json::to_string_pretty(value)
        .map(|text| format!("{text}\n"))
        .map_err(|_| CommandError::Failure("could not render the JSON output".to_owned()))
}

fn admin_failure(error: ModerationAdminError, command: &str) -> CommandError {
    match error {
        ModerationAdminError::InvalidInput => {
            CommandError::Usage(format!("{command}: an argument is out of range"))
        }
        _ => CommandError::Failure(DATABASE_FAILURE.to_owned()),
    }
}

fn report_failure(error: ModerationAdminError, report_id: Uuid, verb: &str) -> CommandError {
    match error {
        ModerationAdminError::ReportNotFound => {
            CommandError::Failure(format!("report {report_id} not found"))
        }
        ModerationAdminError::ReportConflict { current } => CommandError::Failure(format!(
            "report {report_id} is already {}; it cannot be {verb}",
            current.as_str()
        )),
        other => admin_failure(other, "reports"),
    }
}

fn user_failure(error: ModerationError, user_id: Uuid) -> CommandError {
    match error {
        ModerationError::UserNotFound => {
            CommandError::Failure(format!("user {user_id} not found, or already deleted"))
        }
        ModerationError::RequestValidation => CommandError::Usage(
            "--reason must be 1 to 500 characters without control characters".to_owned(),
        ),
        _ => CommandError::Failure(DATABASE_FAILURE.to_owned()),
    }
}

fn account_failure(error: AccountDeletionError, user_id: Uuid) -> CommandError {
    match error {
        AccountDeletionError::AccountNotFound => CommandError::Failure(format!(
            "account {user_id} not found, or its deletion already started"
        )),
        AccountDeletionError::GroupOwnershipTransferRequired => CommandError::Failure(format!(
            "account {user_id} owns an active group; ownership must be transferred before the account can be deleted (the admin CLI cannot transfer ownership)"
        )),
        _ => CommandError::Failure(DATABASE_FAILURE.to_owned()),
    }
}
