use std::time::Duration;

use jamye_server::domain::media::{
    FinalizePolicyError, FinalizedObject, InspectedObject, MAX_AUDIO_DURATION_SECONDS,
    MAX_POSTER_BYTES, MediaKind, MediaScope, POSTER_CONTENT_TYPE, PosterCandidate,
    PosterPolicyError, ValidatedUpload, validate_finalized_object, validate_poster_link,
    validate_upload,
};
use uuid::Uuid;

#[test]
fn exact_image_and_video_metadata_is_authoritative_and_non_audio_drops_duration() {
    let image = expected("image/jpeg", 1_024);
    assert_eq!(
        validate_finalized_object(
            &image,
            &InspectedObject {
                content_type: Some("image/jpeg".to_owned()),
                byte_size: Some(1_024),
                audio_duration: None,
            },
        ),
        Ok(FinalizedObject {
            kind: MediaKind::Image,
            content_type: "image/jpeg".to_owned(),
            byte_size: 1_024,
            duration_seconds: None,
        })
    );

    let video = expected("video/mp4", 4_096);
    assert_eq!(
        validate_finalized_object(
            &video,
            &InspectedObject {
                content_type: Some("video/mp4".to_owned()),
                byte_size: Some(4_096),
                audio_duration: Some(Duration::from_secs(999)),
            },
        ),
        Ok(FinalizedObject {
            kind: MediaKind::Video,
            content_type: "video/mp4".to_owned(),
            byte_size: 4_096,
            duration_seconds: None,
        })
    );
}

#[test]
fn missing_or_different_provider_metadata_is_rejected_without_normalization() {
    let expected = expected("image/png", 2_048);
    for inspected in [
        InspectedObject {
            content_type: None,
            byte_size: Some(2_048),
            audio_duration: None,
        },
        InspectedObject {
            content_type: Some("image/png".to_owned()),
            byte_size: None,
            audio_duration: None,
        },
    ] {
        assert_eq!(
            validate_finalized_object(&expected, &inspected),
            Err(FinalizePolicyError::MetadataMissing)
        );
    }

    assert_eq!(
        validate_finalized_object(
            &expected,
            &InspectedObject {
                content_type: Some("image/png; charset=binary".to_owned()),
                byte_size: Some(2_048),
                audio_duration: None,
            },
        ),
        Err(FinalizePolicyError::ContentTypeMismatch)
    );
    assert_eq!(
        validate_finalized_object(
            &expected,
            &InspectedObject {
                content_type: Some("image/png".to_owned()),
                byte_size: Some(2_049),
                audio_duration: None,
            },
        ),
        Err(FinalizePolicyError::ByteSizeMismatch)
    );
}

#[test]
fn audio_duration_is_required_positive_capped_and_stored_without_underreporting() {
    let expected = expected("audio/ogg", 8_192);
    assert_eq!(
        validate_finalized_object(
            &expected,
            &InspectedObject {
                content_type: Some("audio/ogg".to_owned()),
                byte_size: Some(8_192),
                audio_duration: Some(Duration::from_millis(37_001)),
            },
        ),
        Ok(FinalizedObject {
            kind: MediaKind::Audio,
            content_type: "audio/ogg".to_owned(),
            byte_size: 8_192,
            duration_seconds: Some(38),
        })
    );
    assert_eq!(
        validate_finalized_object(
            &expected,
            &InspectedObject {
                content_type: Some("audio/ogg".to_owned()),
                byte_size: Some(8_192),
                audio_duration: Some(Duration::from_secs(MAX_AUDIO_DURATION_SECONDS)),
            },
        ),
        Ok(FinalizedObject {
            kind: MediaKind::Audio,
            content_type: "audio/ogg".to_owned(),
            byte_size: 8_192,
            duration_seconds: Some(MAX_AUDIO_DURATION_SECONDS),
        })
    );

    for duration in [None, Some(Duration::ZERO)] {
        assert_eq!(
            validate_finalized_object(
                &expected,
                &InspectedObject {
                    content_type: Some("audio/ogg".to_owned()),
                    byte_size: Some(8_192),
                    audio_duration: duration,
                },
            ),
            Err(FinalizePolicyError::AudioDurationInvalid)
        );
    }
    assert_eq!(
        validate_finalized_object(
            &expected,
            &InspectedObject {
                content_type: Some("audio/ogg".to_owned()),
                byte_size: Some(8_192),
                audio_duration: Some(
                    Duration::from_secs(MAX_AUDIO_DURATION_SECONDS) + Duration::from_nanos(1),
                ),
            },
        ),
        Err(FinalizePolicyError::AudioDurationExceeded)
    );
}

fn expected(content_type: &str, byte_size: u64) -> ValidatedUpload {
    let Ok(upload) = validate_upload(MediaScope::Chat, content_type, byte_size, Some("원본 파일"))
    else {
        panic!("test upload policy should be valid");
    };
    upload
}

#[test]
fn valid_poster_link_is_accepted_for_a_confirmed_matching_chat_video() {
    assert_eq!(
        validate_poster_link(&valid_video(), user_id(), target_id(), &valid_poster()),
        Ok(())
    );
}

#[test]
fn poster_link_requires_a_chat_scope_video_upload() {
    assert_eq!(
        validate_poster_link(
            &expected("image/jpeg", 1_024),
            user_id(),
            target_id(),
            &valid_poster()
        ),
        Err(PosterPolicyError::VideoOnly)
    );

    let Ok(topic_video_like) =
        validate_upload(MediaScope::Topic, "image/jpeg", 1_024, Some("원본 파일"))
    else {
        panic!("test topic upload policy should be valid");
    };
    assert_eq!(
        validate_poster_link(&topic_video_like, user_id(), target_id(), &valid_poster()),
        Err(PosterPolicyError::ChatScopeOnly)
    );
}

#[test]
fn poster_link_requires_a_confirmed_chat_scope_jpeg_image_within_size_and_dimension_limits() {
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                kind: MediaKind::Video,
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterNotImage)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                content_type: "image/png".to_owned(),
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterContentType)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                byte_size: MAX_POSTER_BYTES + 1,
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterTooLarge)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                byte_size: MAX_POSTER_BYTES,
                ..valid_poster()
            }
        ),
        Ok(()),
        "boundary byte_size equal to the maximum must be accepted"
    );
}

#[test]
fn poster_link_requires_confirmation_matching_ownership_and_exclusive_use() {
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                status_confirmed: false,
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterNotConfirmed)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                user_id: other_user_id(),
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterOwnerMismatch)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                target_id: other_target_id(),
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterTargetMismatch)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                scope: MediaScope::Topic,
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterTargetMismatch)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                already_linked: true,
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterAlreadyLinked)
    );
    assert_eq!(
        validate_poster_link(
            &valid_video(),
            user_id(),
            target_id(),
            &PosterCandidate {
                has_own_poster: true,
                ..valid_poster()
            }
        ),
        Err(PosterPolicyError::PosterHasPoster)
    );
}

fn valid_video() -> ValidatedUpload {
    let Ok(upload) = validate_upload(MediaScope::Chat, "video/mp4", 4_096, Some("원본 영상.mp4"))
    else {
        panic!("test video upload policy should be valid");
    };
    upload
}

fn valid_poster() -> PosterCandidate {
    PosterCandidate {
        kind: MediaKind::Image,
        content_type: POSTER_CONTENT_TYPE.to_owned(),
        byte_size: 2_048,
        status_confirmed: true,
        scope: MediaScope::Chat,
        target_id: target_id(),
        user_id: user_id(),
        already_linked: false,
        has_own_poster: false,
    }
}

fn user_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn other_user_id() -> Uuid {
    Uuid::from_u128(0xbbbbbbbb_bbbb_4bbb_8bbb_bbbbbbbbbbbb)
}

fn target_id() -> Uuid {
    Uuid::from_u128(0xcccccccc_cccc_4ccc_8ccc_cccccccccccc)
}

fn other_target_id() -> Uuid {
    Uuid::from_u128(0xdddddddd_dddd_4ddd_8ddd_dddddddddddd)
}
