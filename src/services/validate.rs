//! The input checks several services share.

use super::*;

pub(super) fn validate_name(name: &str) -> Result<(), ServiceError> {
    if (1..=200).contains(&name.trim().chars().count()) {
        Ok(())
    } else {
        Err(ServiceError::Invalid)
    }
}

pub(super) fn validate_replay_type(
    receipt: &MutationReceipt,
    expected: &str,
) -> Result<(), ServiceError> {
    if receipt.entity_type == expected {
        Ok(())
    } else {
        Err(ServiceError::Conflict)
    }
}

pub(super) fn validate_username(username: &str) -> Result<(), ServiceError> {
    let username = username.trim();
    if !(3..=64).contains(&username.len())
        || !username.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        Err(ServiceError::Invalid)
    } else {
        Ok(())
    }
}
