//! Group topology, membership, role, and bounded-invite use cases.

use std::{fmt, sync::Arc, time::Duration};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::{
    auth::CredentialSource,
    groups::{
        CreateGroupCommand, CreateInviteCommand, GetGroupQuery, GroupActorCommand, GroupPage,
        GroupRecord, GroupRole, GroupsClock, GroupsRepository, GroupsRepositoryError,
        InviteJoinRecord, InviteRecord, ListGroupsQuery, ListMembersQuery, MemberPage,
        RedeemInviteCommand, RemoveMemberCommand, RenameGroupCommand, SetMemberRoleCommand,
    },
    rate_limit::{RateLimitOutcome, RateLimitRequest, RateLimiter},
    transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
};

pub const DEFAULT_GROUP_MEMBER_CAP: i32 = 12;
pub const DEFAULT_PAGE_LIMIT: u32 = 50;
pub const MAX_PAGE_LIMIT: u32 = 100;
const MAX_INVITE_CODE_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct GroupsService {
    dependencies: GroupsDependencies,
    rate_limits: GroupsRateLimitPolicy,
}

#[derive(Clone)]
pub struct GroupsDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn GroupsRepository>,
    pub rate_limiter: Arc<dyn RateLimiter>,
    pub credentials: Arc<dyn CredentialSource>,
    pub clock: Arc<dyn GroupsClock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupsRateLimitPolicy {
    pub invite_issue: GroupsEndpointRateLimit,
    pub invite_redeem: GroupsEndpointRateLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupsEndpointRateLimit {
    pub limit: u32,
    pub window: Duration,
}

impl GroupsService {
    pub fn new(
        dependencies: GroupsDependencies,
        rate_limits: GroupsRateLimitPolicy,
    ) -> Result<Self, GroupsError> {
        if !rate_limits.is_valid() {
            return Err(GroupsError::InvalidConfiguration);
        }
        Ok(Self {
            dependencies,
            rate_limits,
        })
    }

    pub async fn create_group(
        &self,
        owner_id: Uuid,
        input: GroupCreateInput,
    ) -> Result<GroupRecord, GroupsError> {
        validate_name(&input.name)?;
        let command = CreateGroupCommand {
            group_id: Uuid::new_v4(),
            membership_id: Uuid::new_v4(),
            main_chatroom_id: Uuid::new_v4(),
            owner_id,
            name: input.name,
            max_members: DEFAULT_GROUP_MEMBER_CAP,
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .create_group(transaction.as_mut(), &command)
            .await;
        self.finish(transaction, result).await
    }

    pub async fn list_groups(
        &self,
        user_id: Uuid,
        input: PageInput,
    ) -> Result<GroupPage, GroupsError> {
        let (after, limit) = validate_page(input)?;
        self.dependencies
            .repository
            .list_groups(ListGroupsQuery {
                user_id,
                after,
                limit,
            })
            .await
            .map_err(GroupsError::from)
    }

    pub async fn get_group(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
    ) -> Result<GroupRecord, GroupsError> {
        self.dependencies
            .repository
            .get_group(GetGroupQuery { group_id, actor_id })
            .await
            .map_err(GroupsError::from)
    }

    pub async fn list_members(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        input: PageInput,
    ) -> Result<MemberPage, GroupsError> {
        let (after, limit) = validate_page(input)?;
        self.dependencies
            .repository
            .list_members(ListMembersQuery {
                group_id,
                actor_id,
                after,
                limit,
            })
            .await
            .map_err(GroupsError::from)
    }

    pub async fn rename_group(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        input: GroupPatchInput,
    ) -> Result<GroupRecord, GroupsError> {
        validate_name(&input.name)?;
        let command = RenameGroupCommand {
            group_id,
            actor_id,
            name: input.name,
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .rename_group(transaction.as_mut(), &command)
            .await;
        self.finish(transaction, result).await
    }

    pub async fn delete_group(&self, actor_id: Uuid, group_id: Uuid) -> Result<(), GroupsError> {
        let mut transaction = self.begin().await?;
        let result = self
            .delete_group_in_transaction(transaction.as_mut(), actor_id, group_id)
            .await;
        self.finish_application_result(transaction, result).await
    }

    pub async fn remove_member(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        target_user_id: Uuid,
    ) -> Result<(), GroupsError> {
        let mut transaction = self.begin().await?;
        let result = self
            .remove_member_in_transaction(transaction.as_mut(), actor_id, group_id, target_user_id)
            .await;
        self.finish_application_result(transaction, result).await
    }

    /// Applies the authoritative group soft-delete using a caller-owned transaction.
    ///
    /// Task-6c uses this cohesive task-6 boundary so the mutation and its durable
    /// realtime control intent share the sole application transaction.
    pub async fn delete_group_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        actor_id: Uuid,
        group_id: Uuid,
    ) -> Result<(), GroupsError> {
        let command = GroupActorCommand { group_id, actor_id };
        self.dependencies
            .repository
            .delete_group(transaction, &command)
            .await
            .map_err(GroupsError::from)
    }

    /// Applies member removal or voluntary leave using a caller-owned transaction.
    pub async fn remove_member_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        actor_id: Uuid,
        group_id: Uuid,
        target_user_id: Uuid,
    ) -> Result<(), GroupsError> {
        let command = RemoveMemberCommand {
            group_id,
            actor_id,
            target_user_id,
        };
        self.dependencies
            .repository
            .remove_member(transaction, &command)
            .await
            .map_err(GroupsError::from)
    }

    pub async fn set_member_role(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        target_user_id: Uuid,
        role: GroupRole,
    ) -> Result<(), GroupsError> {
        let command = SetMemberRoleCommand {
            group_id,
            actor_id,
            target_user_id,
            role,
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .set_member_role(transaction.as_mut(), &command)
            .await;
        self.finish(transaction, result).await
    }

    pub async fn create_invite(
        &self,
        actor_id: Uuid,
        group_id: Uuid,
        input: InviteCreateInput,
        rate_limit_subject: &str,
    ) -> Result<InviteRecord, GroupsError> {
        validate_invite(&input, self.dependencies.clock.now())?;
        self.check_rate_limit(
            "invite_issue",
            rate_limit_subject,
            &self.rate_limits.invite_issue,
        )
        .await?;

        for _ in 0..MAX_INVITE_CODE_ATTEMPTS {
            let credential = self
                .dependencies
                .credentials
                .generate()
                .map_err(|_| GroupsError::CredentialUnavailable)?;
            let command = CreateInviteCommand {
                invite_id: Uuid::new_v4(),
                group_id,
                actor_id,
                code: credential.raw.into_string(),
                expires_at: input.expires_at,
                max_uses: input.max_uses,
            };
            let mut transaction = self.begin().await?;
            let result = self
                .dependencies
                .repository
                .create_invite(transaction.as_mut(), &command)
                .await;
            if matches!(&result, Err(GroupsRepositoryError::InviteCodeCollision)) {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| GroupsError::DatabaseUnavailable)?;
                continue;
            }
            return self.finish(transaction, result).await;
        }
        Err(GroupsError::CredentialUnavailable)
    }

    pub async fn redeem_invite(
        &self,
        actor_id: Uuid,
        code: String,
        rate_limit_subject: &str,
    ) -> Result<InviteJoinRecord, GroupsError> {
        validate_invite_code(&code)?;
        self.check_rate_limit(
            "invite_redeem",
            rate_limit_subject,
            &self.rate_limits.invite_redeem,
        )
        .await?;
        let command = RedeemInviteCommand {
            membership_id: Uuid::new_v4(),
            actor_id,
            code,
            now: self.dependencies.clock.now(),
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .redeem_invite(transaction.as_mut(), &command)
            .await;
        self.finish(transaction, result).await
    }

    async fn check_rate_limit(
        &self,
        endpoint: &'static str,
        subject: &str,
        policy: &GroupsEndpointRateLimit,
    ) -> Result<(), GroupsError> {
        match self
            .dependencies
            .rate_limiter
            .check(&RateLimitRequest {
                endpoint,
                subject: subject.to_owned(),
                limit: policy.limit,
                window: policy.window,
            })
            .await
            .map_err(|_| GroupsError::RateLimitUnavailable)?
        {
            RateLimitOutcome::Allowed => Ok(()),
            RateLimitOutcome::Denied { retry_after } => {
                Err(GroupsError::RateLimited { retry_after })
            }
        }
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, GroupsError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| GroupsError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, GroupsRepositoryError>,
    ) -> Result<T, GroupsError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| GroupsError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| GroupsError::DatabaseUnavailable)?;
                Err(error.into())
            }
        }
    }

    async fn finish_application_result<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, GroupsError>,
    ) -> Result<T, GroupsError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| GroupsError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| GroupsError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

impl GroupsRateLimitPolicy {
    fn is_valid(&self) -> bool {
        [&self.invite_issue, &self.invite_redeem]
            .into_iter()
            .all(|policy| policy.limit > 0 && !policy.window.is_zero())
    }
}

fn validate_name(name: &str) -> Result<(), GroupsError> {
    if name.is_empty() || name.chars().count() > 128 {
        return Err(GroupsError::RequestValidation);
    }
    Ok(())
}

fn validate_page(input: PageInput) -> Result<(Option<Uuid>, u32), GroupsError> {
    let limit = input.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(GroupsError::RequestValidation);
    }
    let after = input
        .after
        .map(|cursor| Uuid::try_parse(&cursor))
        .transpose()
        .map_err(|_| GroupsError::RequestValidation)?;
    Ok((after, limit))
}

fn validate_invite(input: &InviteCreateInput, now: OffsetDateTime) -> Result<(), GroupsError> {
    if input.max_uses.is_some_and(|uses| uses <= 0)
        || input.expires_at.is_some_and(|expires_at| expires_at <= now)
    {
        return Err(GroupsError::RequestValidation);
    }
    Ok(())
}

fn validate_invite_code(code: &str) -> Result<(), GroupsError> {
    if !(16..=64).contains(&code.len())
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(GroupsError::RequestValidation);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupCreateInput {
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupPatchInput {
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageInput {
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InviteCreateInput {
    pub expires_at: Option<OffsetDateTime>,
    pub max_uses: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupsError {
    RequestValidation,
    GroupNotFound,
    MembershipRequired,
    OwnerRequired,
    MemberNotFound,
    OwnerConflict,
    GroupFull,
    InviteNotFound,
    InviteExpired,
    InviteExhausted,
    TopologyConflict,
    RateLimited { retry_after: Duration },
    RateLimitUnavailable,
    CredentialUnavailable,
    DatabaseUnavailable,
    InvalidConfiguration,
}

impl From<GroupsRepositoryError> for GroupsError {
    fn from(error: GroupsRepositoryError) -> Self {
        match error {
            GroupsRepositoryError::GroupNotFound => Self::GroupNotFound,
            GroupsRepositoryError::MembershipRequired => Self::MembershipRequired,
            GroupsRepositoryError::OwnerRequired => Self::OwnerRequired,
            GroupsRepositoryError::MemberNotFound => Self::MemberNotFound,
            GroupsRepositoryError::OwnerConflict => Self::OwnerConflict,
            GroupsRepositoryError::GroupFull => Self::GroupFull,
            GroupsRepositoryError::InviteNotFound => Self::InviteNotFound,
            GroupsRepositoryError::InviteExpired => Self::InviteExpired,
            GroupsRepositoryError::InviteExhausted => Self::InviteExhausted,
            GroupsRepositoryError::TopologyConflict => Self::TopologyConflict,
            GroupsRepositoryError::InviteCodeCollision
            | GroupsRepositoryError::MembershipConflict
            | GroupsRepositoryError::InvalidData
            | GroupsRepositoryError::Unavailable => Self::DatabaseUnavailable,
        }
    }
}

impl fmt::Display for GroupsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("group operation failed")
    }
}

impl std::error::Error for GroupsError {}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemGroupsClock;

impl GroupsClock for SystemGroupsClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}
