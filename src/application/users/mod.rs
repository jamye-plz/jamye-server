//! Authenticated current-user profile use cases.

use std::{fmt, sync::Arc};

use uuid::Uuid;

use crate::ports::{
    auth::{AuthRepository, AvatarPatch, ProfilePatch, UserProfile},
    transactions::{BoxTransactionHandle, TransactionManager},
};

#[derive(Clone)]
pub struct UserService {
    transactions: Arc<dyn TransactionManager>,
    repository: Arc<dyn AuthRepository>,
}

impl UserService {
    pub fn new(
        transactions: Arc<dyn TransactionManager>,
        repository: Arc<dyn AuthRepository>,
    ) -> Self {
        Self {
            transactions,
            repository,
        }
    }

    pub async fn get(&self, user_id: Uuid) -> Result<UserProfile, UserError> {
        self.repository
            .profile(user_id)
            .await
            .map_err(|_| UserError::DatabaseUnavailable)?
            .ok_or(UserError::ProfileNotFound)
    }

    pub async fn update(&self, user_id: Uuid, input: UserPatch) -> Result<UserProfile, UserError> {
        let patch = validate_patch(input)?;
        let mut transaction = self.begin().await?;
        let result = self
            .repository
            .update_profile(transaction.as_mut(), user_id, &patch)
            .await;
        match result {
            Ok(Some(profile)) => {
                self.transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| UserError::DatabaseUnavailable)?;
                Ok(profile)
            }
            Ok(None) => {
                self.rollback_with(transaction, UserError::ProfileNotFound)
                    .await
            }
            Err(_) => {
                self.rollback_with(transaction, UserError::DatabaseUnavailable)
                    .await
            }
        }
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, UserError> {
        self.transactions
            .begin()
            .await
            .map_err(|_| UserError::DatabaseUnavailable)
    }

    async fn rollback_with<T>(
        &self,
        transaction: BoxTransactionHandle,
        error: UserError,
    ) -> Result<T, UserError> {
        self.transactions
            .rollback(transaction)
            .await
            .map_err(|_| UserError::DatabaseUnavailable)?;
        Err(error)
    }
}

fn validate_patch(input: UserPatch) -> Result<ProfilePatch, UserError> {
    let nickname = match input.nickname {
        PatchValue::Value(value) if !value.is_empty() && value.chars().count() <= 64 => Some(value),
        PatchValue::Value(_) => return Err(UserError::RequestValidation),
        PatchValue::Omitted | PatchValue::Null => None,
    };
    let avatar_url = match input.avatar_url {
        PatchValue::Value(value) if value.is_empty() => AvatarPatch::Clear,
        PatchValue::Value(value) if value.chars().count() <= 512 => AvatarPatch::Set(value),
        PatchValue::Value(_) => return Err(UserError::RequestValidation),
        PatchValue::Omitted | PatchValue::Null => AvatarPatch::Unchanged,
    };
    Ok(ProfilePatch {
        nickname,
        avatar_url,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserPatch {
    pub nickname: PatchValue<String>,
    pub avatar_url: PatchValue<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchValue<T> {
    Omitted,
    Null,
    Value(T),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserError {
    RequestValidation,
    ProfileNotFound,
    DatabaseUnavailable,
}

impl fmt::Display for UserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("profile operation failed")
    }
}

impl std::error::Error for UserError {}
