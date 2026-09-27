//! Profile-related domain validation shared by account updates and auth ingress.

use url::Url;

pub const MAX_AVATAR_URL_CHARS: usize = 512;
const HTTPS_PREFIX: &[u8] = b"https://";

pub fn valid_avatar_url(value: &str) -> bool {
    if value.is_empty()
        || value.chars().count() > MAX_AVATAR_URL_CHARS
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return false;
    }
    let bytes = value.as_bytes();
    let Some(prefix) = bytes.get(..HTTPS_PREFIX.len()) else {
        return false;
    };
    if prefix != HTTPS_PREFIX {
        return false;
    }
    let Some(first_authority_byte) = bytes.get(HTTPS_PREFIX.len()) else {
        return false;
    };
    if matches!(first_authority_byte, b'/' | b'?' | b'#') {
        return false;
    }
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    url.scheme() == "https" && url.has_host()
}

pub fn normalize_provider_avatar_url(value: Option<String>) -> Option<String> {
    let value = value.filter(|value| !value.is_empty())?;
    let candidate = match Url::parse(&value) {
        Ok(mut url) if url.scheme() == "http" => {
            url.set_scheme("https").ok()?;
            url.to_string()
        }
        Ok(_) => value,
        Err(_) => return None,
    };
    valid_avatar_url(&candidate).then_some(candidate)
}
