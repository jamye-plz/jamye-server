use jamye_server::domain::media::{
    MAX_MEDIA_PER_MESSAGE, MediaKind, MessageCompositionKind, MessageMediaCandidate,
    MessageMediaPolicyError, OrderedMessageMedia, ValidatedMessageComposition,
    validate_message_composition,
};
use uuid::Uuid;

#[test]
fn missing_null_or_empty_body_without_media_is_rejected_exactly() {
    for body in [None, Some("")] {
        assert_eq!(
            validate_message_composition(body, &[]),
            Err(MessageMediaPolicyError::MessageContentRequired)
        );
    }
}

#[test]
fn ordinary_text_and_up_to_four_ordered_visual_attachments_are_accepted() {
    assert_eq!(
        validate_message_composition(Some("   "), &[]),
        Ok(ValidatedMessageComposition {
            kind: MessageCompositionKind::Standard,
            media: Vec::new(),
        })
    );

    let media = vec![
        candidate(MediaKind::Image, 1, "image-one"),
        candidate(MediaKind::Video, 2, "video-two"),
        candidate(MediaKind::Image, 3, "image-three"),
        candidate(MediaKind::Image, 4, "image-four"),
    ];
    assert_eq!(media.len(), MAX_MEDIA_PER_MESSAGE);
    assert_eq!(
        validate_message_composition(Some(""), &media),
        Ok(ValidatedMessageComposition {
            kind: MessageCompositionKind::Standard,
            media: media
                .iter()
                .enumerate()
                .map(|(position, candidate)| OrderedMessageMedia {
                    upload_id: candidate.upload_id,
                    object_key: candidate.object_key.clone(),
                    kind: candidate.kind,
                    position: position as u8,
                })
                .collect(),
        })
    );
}

#[test]
fn attachment_count_and_duplicate_identity_are_rejected_before_binding() {
    let five = (0..=MAX_MEDIA_PER_MESSAGE)
        .map(|index| {
            candidate(
                MediaKind::Image,
                index as u128 + 1,
                &format!("image-{index}"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        validate_message_composition(Some("본문"), &five),
        Err(MessageMediaPolicyError::TooManyAttachments)
    );

    let upload = candidate(MediaKind::Image, 10, "first");
    let duplicate_upload = MessageMediaCandidate {
        upload_id: upload.upload_id,
        object_key: "chat/target/second".to_owned(),
        kind: MediaKind::Image,
    };
    assert_eq!(
        validate_message_composition(Some("본문"), &[upload, duplicate_upload]),
        Err(MessageMediaPolicyError::DuplicateUpload)
    );

    let first = candidate(MediaKind::Image, 11, "same-key");
    let duplicate_key = MessageMediaCandidate {
        upload_id: uuid(12),
        object_key: first.object_key.clone(),
        kind: MediaKind::Video,
    };
    assert_eq!(
        validate_message_composition(Some("본문"), &[first, duplicate_key]),
        Err(MessageMediaPolicyError::DuplicateObjectKey)
    );
}

#[test]
fn voice_is_bodyless_exactly_one_audio_and_never_mixed() {
    let audio = candidate(MediaKind::Audio, 20, "voice");
    assert_eq!(
        validate_message_composition(None, std::slice::from_ref(&audio)),
        Ok(ValidatedMessageComposition {
            kind: MessageCompositionKind::Voice,
            media: vec![OrderedMessageMedia {
                upload_id: audio.upload_id,
                object_key: audio.object_key.clone(),
                kind: MediaKind::Audio,
                position: 0,
            }],
        })
    );
    assert_eq!(
        validate_message_composition(Some(""), std::slice::from_ref(&audio)),
        validate_message_composition(None, std::slice::from_ref(&audio))
    );

    assert_eq!(
        validate_message_composition(Some("음성 설명"), std::slice::from_ref(&audio)),
        Err(MessageMediaPolicyError::InvalidVoiceComposition)
    );
    assert_eq!(
        validate_message_composition(
            None,
            &[
                audio.clone(),
                candidate(MediaKind::Audio, 21, "second-voice")
            ],
        ),
        Err(MessageMediaPolicyError::InvalidVoiceComposition)
    );
    assert_eq!(
        validate_message_composition(
            None,
            &[audio, candidate(MediaKind::Image, 22, "mixed-image")],
        ),
        Err(MessageMediaPolicyError::InvalidVoiceComposition)
    );
}

fn candidate(kind: MediaKind, id: u128, key_suffix: &str) -> MessageMediaCandidate {
    MessageMediaCandidate {
        upload_id: uuid(id),
        object_key: format!("chat/target/{key_suffix}"),
        kind,
    }
}

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
