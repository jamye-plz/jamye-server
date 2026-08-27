//! Framework-free media upload policy shared by presign and finalize.

use std::{collections::HashSet, time::Duration};

use uuid::Uuid;

pub const IMAGE_CONTENT_TYPES: [&str; 4] = ["image/jpeg", "image/png", "image/webp", "image/gif"];
pub const VIDEO_CONTENT_TYPES: [&str; 1] = ["video/mp4"];
pub const AUDIO_CONTENT_TYPES: [&str; 3] = ["audio/webm", "audio/mp4", "audio/ogg"];

pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_AUDIO_BYTES: u64 = 15 * 1024 * 1024;
pub const MAX_AUDIO_DURATION_SECONDS: u64 = 330;
pub const MAX_MEDIA_PER_MESSAGE: usize = 4;
pub const MAX_FILENAME_CHARS: usize = 255;
pub const PRESIGNED_PUT_TTL_SECONDS: u64 = 3_600;
pub const PRESIGNED_GET_TTL_SECONDS: u64 = 600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaScope {
    Chat,
    Topic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageMediaCandidate {
    pub upload_id: Uuid,
    pub object_key: String,
    pub kind: MediaKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderedMessageMedia {
    pub upload_id: Uuid,
    pub object_key: String,
    pub kind: MediaKind,
    pub position: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageCompositionKind {
    Standard,
    Voice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedMessageComposition {
    pub kind: MessageCompositionKind,
    pub media: Vec<OrderedMessageMedia>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageMediaPolicyError {
    MessageContentRequired,
    TooManyAttachments,
    DuplicateUpload,
    DuplicateObjectKey,
    InvalidVoiceComposition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedUpload {
    pub scope: MediaScope,
    pub kind: MediaKind,
    pub content_type: String,
    pub byte_size: u64,
    pub filename: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadPolicyError {
    UnsupportedContentType,
    InvalidByteSize,
    FilenameTooLong,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectedObject {
    pub content_type: Option<String>,
    pub byte_size: Option<u64>,
    pub audio_duration: Option<Duration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedObject {
    pub kind: MediaKind,
    pub content_type: String,
    pub byte_size: u64,
    pub duration_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinalizePolicyError {
    MetadataMissing,
    ContentTypeMismatch,
    ByteSizeMismatch,
    AudioDurationInvalid,
    AudioDurationExceeded,
}

/// Validate the policy shared by upload-intent creation and authoritative object finalization.
pub fn validate_upload(
    scope: MediaScope,
    content_type: &str,
    byte_size: u64,
    filename: Option<&str>,
) -> Result<ValidatedUpload, UploadPolicyError> {
    let kind = classify_content_type(scope, content_type)
        .ok_or(UploadPolicyError::UnsupportedContentType)?;
    let max_bytes = match kind {
        MediaKind::Image => MAX_IMAGE_BYTES,
        MediaKind::Video => MAX_VIDEO_BYTES,
        MediaKind::Audio => MAX_AUDIO_BYTES,
    };

    if byte_size == 0 || byte_size > max_bytes {
        return Err(UploadPolicyError::InvalidByteSize);
    }
    if filename.is_some_and(|value| value.chars().count() > MAX_FILENAME_CHARS) {
        return Err(UploadPolicyError::FilenameTooLong);
    }

    Ok(ValidatedUpload {
        scope,
        kind,
        content_type: content_type.to_owned(),
        byte_size,
        filename: filename.map(ToOwned::to_owned),
    })
}

/// Validate provider-observed metadata before any authoritative finalize mutation.
pub fn validate_finalized_object(
    expected: &ValidatedUpload,
    inspected: &InspectedObject,
) -> Result<FinalizedObject, FinalizePolicyError> {
    let content_type = inspected
        .content_type
        .as_deref()
        .ok_or(FinalizePolicyError::MetadataMissing)?;
    let byte_size = inspected
        .byte_size
        .ok_or(FinalizePolicyError::MetadataMissing)?;

    if content_type != expected.content_type {
        return Err(FinalizePolicyError::ContentTypeMismatch);
    }
    if byte_size != expected.byte_size {
        return Err(FinalizePolicyError::ByteSizeMismatch);
    }

    let duration_seconds = if expected.kind == MediaKind::Audio {
        let duration = inspected
            .audio_duration
            .filter(|duration| !duration.is_zero())
            .ok_or(FinalizePolicyError::AudioDurationInvalid)?;
        if duration > Duration::from_secs(MAX_AUDIO_DURATION_SECONDS) {
            return Err(FinalizePolicyError::AudioDurationExceeded);
        }
        Some(duration.as_secs() + u64::from(duration.subsec_nanos() != 0))
    } else {
        None
    };

    Ok(FinalizedObject {
        kind: expected.kind,
        content_type: content_type.to_owned(),
        byte_size,
        duration_seconds,
    })
}

/// Mint a server-owned key whose namespace binds it to the authorized target scope.
pub fn mint_object_key(scope: MediaScope, target_id: Uuid, upload_id: Uuid) -> String {
    let prefix = match scope {
        MediaScope::Chat => "chat",
        MediaScope::Topic => "topics",
    };
    format!("{prefix}/{target_id}/{upload_id}")
}

/// Build the response override bound into an MD5 presigned download URL.
pub fn download_content_disposition(
    media_id: Uuid,
    content_type: &str,
    stored_filename: Option<&str>,
) -> String {
    let generated = generated_download_filename(media_id, content_type);
    let filename = stored_filename
        .and_then(sanitize_download_filename)
        .unwrap_or_else(|| generated.clone());
    let fallback = if filename.is_ascii() {
        ascii_download_fallback(&filename)
    } else {
        generated
    };
    format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{}",
        encode_rfc_5987(&filename)
    )
}

fn generated_download_filename(media_id: Uuid, content_type: &str) -> String {
    let extension = match content_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "audio/webm" => "webm",
        "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        _ => "bin",
    };
    format!("jamye-{media_id}.{extension}")
}

fn sanitize_download_filename(filename: &str) -> Option<String> {
    let leaf = filename
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())?;
    let sanitized = leaf
        .chars()
        .filter(|character| !character.is_control() && *character != '"')
        .collect::<String>();
    let sanitized = sanitized.trim().trim_start_matches('.').trim();
    (!sanitized.is_empty()).then(|| sanitized.to_owned())
}

fn ascii_download_fallback(filename: &str) -> String {
    filename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '.' | '-' | '_' | '(' | ')')
            {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn encode_rfc_5987(filename: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(filename.len());
    for byte in filename.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

/// Validate message content and attachment identity before any binding side effect.
pub fn validate_message_composition(
    body: Option<&str>,
    media: &[MessageMediaCandidate],
) -> Result<ValidatedMessageComposition, MessageMediaPolicyError> {
    if media.len() > MAX_MEDIA_PER_MESSAGE {
        return Err(MessageMediaPolicyError::TooManyAttachments);
    }

    let mut upload_ids = HashSet::with_capacity(media.len());
    let mut object_keys = HashSet::with_capacity(media.len());
    for candidate in media {
        if !upload_ids.insert(candidate.upload_id) {
            return Err(MessageMediaPolicyError::DuplicateUpload);
        }
        if !object_keys.insert(candidate.object_key.as_str()) {
            return Err(MessageMediaPolicyError::DuplicateObjectKey);
        }
    }

    let body_present = body.is_some_and(|value| !value.is_empty());
    if !body_present && media.is_empty() {
        return Err(MessageMediaPolicyError::MessageContentRequired);
    }

    let audio_count = media
        .iter()
        .filter(|candidate| candidate.kind == MediaKind::Audio)
        .count();
    let kind = match audio_count {
        0 => MessageCompositionKind::Standard,
        1 if !body_present && media.len() == 1 => MessageCompositionKind::Voice,
        _ => return Err(MessageMediaPolicyError::InvalidVoiceComposition),
    };
    let media = media
        .iter()
        .enumerate()
        .map(|(position, candidate)| OrderedMessageMedia {
            upload_id: candidate.upload_id,
            object_key: candidate.object_key.clone(),
            kind: candidate.kind,
            position: position as u8,
        })
        .collect();

    Ok(ValidatedMessageComposition { kind, media })
}

fn classify_content_type(scope: MediaScope, content_type: &str) -> Option<MediaKind> {
    if IMAGE_CONTENT_TYPES.contains(&content_type) {
        return Some(MediaKind::Image);
    }
    if scope == MediaScope::Chat && VIDEO_CONTENT_TYPES.contains(&content_type) {
        return Some(MediaKind::Video);
    }
    if scope == MediaScope::Chat && AUDIO_CONTENT_TYPES.contains(&content_type) {
        return Some(MediaKind::Audio);
    }
    None
}
