//! Framework-free avatar upload policy and object-key / public-URL scheme.

use uuid::Uuid;

pub const AVATAR_CONTENT_TYPE: &str = "image/jpeg";
pub const MAX_AVATAR_BYTES: u64 = 1024 * 1024;
pub const AVATAR_PUT_TTL_SECONDS: u64 = 900;
pub const AVATAR_PUBLIC_PATH_PREFIX: &str = "/api/v1/avatars/";

const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AvatarPolicyError {
    UnsupportedContentType,
    InvalidByteSize,
}

/// Validate the declared upload: exactly `image/jpeg`, 1 byte up to 1 MiB.
pub fn validate_avatar_upload(content_type: &str, byte_size: u64) -> Result<(), AvatarPolicyError> {
    if content_type != AVATAR_CONTENT_TYPE {
        return Err(AvatarPolicyError::UnsupportedContentType);
    }
    if byte_size == 0 || byte_size > MAX_AVATAR_BYTES {
        return Err(AvatarPolicyError::InvalidByteSize);
    }
    Ok(())
}

/// Server-owned private object key. The user id never appears in the public URL.
pub fn mint_avatar_object_key(user_id: Uuid, upload_id: Uuid) -> String {
    format!("avatar/{user_id}/{upload_id}")
}

/// Public, unguessable avatar URL derived only from the upload id.
pub fn avatar_public_url(public_base_url: &str, upload_id: Uuid) -> String {
    format!(
        "{}{AVATAR_PUBLIC_PATH_PREFIX}{upload_id}",
        public_base_url.trim_end_matches('/')
    )
}

pub fn has_jpeg_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(&JPEG_SIGNATURE)
}
