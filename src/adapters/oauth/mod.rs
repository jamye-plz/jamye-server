//! Fixed-origin OAuth and production token adapters.

mod credentials;
mod providers;
mod token;

pub use credentials::OsCredentialSource;
pub use providers::{
    GOOGLE_AUTHORIZE_URL, GOOGLE_IDENTITY_URL, GOOGLE_ISSUER, GOOGLE_JWKS_URL, GOOGLE_TOKEN_URL,
    GoogleIdTokenVerifier, GoogleOAuthProvider, KAKAO_AUTHORIZE_URL, KAKAO_IDENTITY_URL,
    KAKAO_TOKEN_URL, KakaoOAuthProvider, OAuthClientConfig,
};
pub use token::{ProductionTokenCodec, ProductionTokenConfigError};
