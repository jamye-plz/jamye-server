use jamye_server::domain::media::{
    AUDIO_CONTENT_TYPES, IMAGE_CONTENT_TYPES, MAX_AUDIO_BYTES, MAX_AUDIO_DURATION_SECONDS,
    MAX_FILENAME_CHARS, MAX_IMAGE_BYTES, MAX_MEDIA_PER_MESSAGE, MAX_VIDEO_BYTES, MediaKind,
    MediaScope, PRESIGNED_GET_TTL_SECONDS, PRESIGNED_PUT_TTL_SECONDS, UploadPolicyError,
    VIDEO_CONTENT_TYPES, ValidatedUpload, mint_object_key, validate_upload,
};
use uuid::Uuid;

#[test]
fn chat_policy_accepts_the_exact_allowlist_at_each_cap_and_rejects_boundaries() {
    let accepted = [
        ("image/jpeg", MediaKind::Image, MAX_IMAGE_BYTES),
        ("image/png", MediaKind::Image, MAX_IMAGE_BYTES),
        ("image/webp", MediaKind::Image, MAX_IMAGE_BYTES),
        ("image/gif", MediaKind::Image, MAX_IMAGE_BYTES),
        ("video/mp4", MediaKind::Video, MAX_VIDEO_BYTES),
        ("audio/webm", MediaKind::Audio, MAX_AUDIO_BYTES),
        ("audio/mp4", MediaKind::Audio, MAX_AUDIO_BYTES),
        ("audio/ogg", MediaKind::Audio, MAX_AUDIO_BYTES),
    ];

    for (content_type, kind, byte_size) in accepted {
        assert_eq!(
            validate_upload(MediaScope::Chat, content_type, byte_size, None),
            Ok(ValidatedUpload {
                scope: MediaScope::Chat,
                kind,
                content_type: content_type.to_owned(),
                byte_size,
                filename: None,
            })
        );
        assert_eq!(
            validate_upload(MediaScope::Chat, content_type, byte_size + 1, None),
            Err(UploadPolicyError::InvalidByteSize)
        );
    }

    assert_eq!(
        validate_upload(MediaScope::Chat, "image/jpeg", 0, None),
        Err(UploadPolicyError::InvalidByteSize)
    );
    for rejected in [
        "IMAGE/JPEG",
        "image/jpeg; charset=binary",
        " image/jpeg",
        "application/pdf",
    ] {
        assert_eq!(
            validate_upload(MediaScope::Chat, rejected, 1, None),
            Err(UploadPolicyError::UnsupportedContentType)
        );
    }
}

#[test]
fn topic_policy_accepts_only_exact_image_types_at_the_image_cap() {
    for content_type in IMAGE_CONTENT_TYPES {
        assert_eq!(
            validate_upload(MediaScope::Topic, content_type, MAX_IMAGE_BYTES, None),
            Ok(ValidatedUpload {
                scope: MediaScope::Topic,
                kind: MediaKind::Image,
                content_type: content_type.to_owned(),
                byte_size: MAX_IMAGE_BYTES,
                filename: None,
            })
        );
    }

    for content_type in VIDEO_CONTENT_TYPES.into_iter().chain(AUDIO_CONTENT_TYPES) {
        assert_eq!(
            validate_upload(MediaScope::Topic, content_type, 1, None),
            Err(UploadPolicyError::UnsupportedContentType)
        );
    }
}

#[test]
fn filename_policy_preserves_unicode_and_special_characters_with_a_255_character_cap() {
    let filename = " 여름/기록 \"최종\".jpg ";
    assert_eq!(
        validate_upload(MediaScope::Chat, "image/jpeg", 1, Some(filename),),
        Ok(ValidatedUpload {
            scope: MediaScope::Chat,
            kind: MediaKind::Image,
            content_type: "image/jpeg".to_owned(),
            byte_size: 1,
            filename: Some(filename.to_owned()),
        })
    );

    let exact_limit = "가".repeat(MAX_FILENAME_CHARS);
    assert!(validate_upload(MediaScope::Chat, "image/jpeg", 1, Some(&exact_limit),).is_ok());

    let over_limit = "가".repeat(MAX_FILENAME_CHARS + 1);
    assert_eq!(
        validate_upload(MediaScope::Chat, "image/jpeg", 1, Some(&over_limit),),
        Err(UploadPolicyError::FilenameTooLong)
    );
}

#[test]
fn server_mints_exact_scope_bound_keys_from_ids_without_client_path_input() {
    let target_id = Uuid::from_u128(0x11111111_1111_4111_8111_111111111111);
    let upload_id = Uuid::from_u128(0x22222222_2222_4222_8222_222222222222);

    assert_eq!(
        mint_object_key(MediaScope::Chat, target_id, upload_id),
        "chat/11111111-1111-4111-8111-111111111111/22222222-2222-4222-8222-222222222222"
    );
    assert_eq!(
        mint_object_key(MediaScope::Topic, target_id, upload_id),
        "topics/11111111-1111-4111-8111-111111111111/22222222-2222-4222-8222-222222222222"
    );
}

#[test]
fn media_limits_and_short_presign_ttls_remain_locked() {
    assert_eq!(IMAGE_CONTENT_TYPES.len(), 4);
    assert_eq!(VIDEO_CONTENT_TYPES.len(), 1);
    assert_eq!(AUDIO_CONTENT_TYPES.len(), 3);
    assert_eq!(MAX_IMAGE_BYTES, 10_485_760);
    assert_eq!(MAX_VIDEO_BYTES, 52_428_800);
    assert_eq!(MAX_AUDIO_BYTES, 15_728_640);
    assert_eq!(MAX_AUDIO_DURATION_SECONDS, 330);
    assert_eq!(MAX_MEDIA_PER_MESSAGE, 4);
    assert_eq!(PRESIGNED_PUT_TTL_SECONDS, 3_600);
    assert_eq!(PRESIGNED_GET_TTL_SECONDS, 600);
}
