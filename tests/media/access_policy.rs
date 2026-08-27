use jamye_server::domain::media::download_content_disposition;
use uuid::Uuid;

const MEDIA_ID: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn missing_filename_uses_a_stable_allowlisted_mime_extension() {
    let media_id = media_id();
    for (content_type, extension) in [
        ("image/jpeg", "jpg"),
        ("image/png", "png"),
        ("image/webp", "webp"),
        ("image/gif", "gif"),
        ("video/mp4", "mp4"),
        ("audio/webm", "webm"),
        ("audio/mp4", "m4a"),
        ("audio/ogg", "ogg"),
        ("application/x-unknown", "bin"),
    ] {
        let filename = format!("jamye-{MEDIA_ID}.{extension}");
        assert_eq!(
            download_content_disposition(media_id, content_type, None),
            format!("attachment; filename=\"{filename}\"; filename*=UTF-8''{filename}")
        );
    }
}

#[test]
fn unicode_filename_uses_rfc_5987_and_an_ascii_fallback() {
    let media_id = media_id();
    assert_eq!(
        download_content_disposition(media_id, "image/jpeg", Some("여름 기록 (최종).jpg"),),
        concat!(
            "attachment; filename=\"jamye-11111111-1111-4111-8111-111111111111.jpg\"; ",
            "filename*=UTF-8''%EC%97%AC%EB%A6%84%20%EA%B8%B0%EB%A1%9D%20",
            "%28%EC%B5%9C%EC%A2%85%29.jpg"
        )
    );
}

#[test]
fn unsafe_path_header_and_traversal_characters_are_removed() {
    let media_id = media_id();
    let disposition = download_content_disposition(
        media_id,
        "image/png",
        Some("..\\..//evil\r\n\"; filename=\"owned.png"),
    );

    assert_eq!(
        disposition,
        concat!(
            "attachment; filename=\"evil_ filename_owned.png\"; ",
            "filename*=UTF-8''evil%3B%20filename%3Downed.png"
        )
    );
    assert!(!disposition.contains('\r'));
    assert!(!disposition.contains('\n'));
    assert!(!disposition.contains("../"));
    assert!(!disposition.contains("..\\"));
}

fn media_id() -> Uuid {
    Uuid::from_u128(0x11111111_1111_4111_8111_111111111111)
}
