use std::time::Duration;

use jamye_server::domain::media::{
    FinalizePolicyError, FinalizedObject, InspectedObject, MAX_AUDIO_DURATION_SECONDS, MediaKind,
    MediaScope, ValidatedUpload, validate_finalized_object, validate_upload,
};

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
