use std::{collections::HashMap, time::Duration};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::{Client, redirect::Policy};
use serde::Deserialize;
use url::Url;

use crate::ports::oauth_provider::{
    AuthorizationRequest, OAuthProvider, OAuthProviderError, OAuthProviderFuture,
    ProviderExchangeRequest, ProviderIdentity, ProviderKind,
};

pub const KAKAO_AUTHORIZE_URL: &str = "https://kauth.kakao.com/oauth/authorize";
pub const KAKAO_TOKEN_URL: &str = "https://kauth.kakao.com/oauth/token";
pub const KAKAO_IDENTITY_URL: &str = "https://kapi.kakao.com/v2/user/me";
pub const GOOGLE_ISSUER: &str = "https://accounts.google.com";
pub const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_IDENTITY_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
pub const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

#[derive(Clone)]
pub struct OAuthClientConfig {
    client_id: String,
    client_secret: String,
    timeout: Duration,
}

impl OAuthClientConfig {
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, OAuthProviderError> {
        let client_id = client_id.into();
        let client_secret = client_secret.into();
        if client_id.trim().is_empty()
            || client_secret.trim().is_empty()
            || timeout.is_zero()
            || timeout > Duration::from_secs(30)
        {
            return Err(OAuthProviderError::InvalidConfiguration);
        }
        Ok(Self {
            client_id,
            client_secret,
            timeout,
        })
    }
}

pub struct KakaoOAuthProvider {
    config: OAuthClientConfig,
    client: Client,
}

impl KakaoOAuthProvider {
    pub fn new(config: OAuthClientConfig) -> Result<Self, OAuthProviderError> {
        let client = oauth_client(config.timeout)?;
        Ok(Self { config, client })
    }

    async fn exchange_identity(
        &self,
        request: &ProviderExchangeRequest,
    ) -> Result<ProviderIdentity, OAuthProviderError> {
        let token = self
            .client
            .post(KAKAO_TOKEN_URL)
            .form(&KakaoTokenRequest {
                grant_type: "authorization_code",
                client_id: &self.config.client_id,
                client_secret: &self.config.client_secret,
                redirect_uri: &request.redirect_uri,
                code: &request.authorization_code,
                code_verifier: &request.code_verifier,
            })
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| provider_unavailable("kakao_token"))?
            .json::<OAuthAccessToken>()
            .await
            .map_err(|_| provider_unavailable("kakao_token_decode"))?;
        let profile = self
            .client
            .get(KAKAO_IDENTITY_URL)
            .bearer_auth(&token.access_token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| provider_unavailable("kakao_identity"))?
            .json::<KakaoIdentity>()
            .await
            .map_err(|_| OAuthProviderError::InvalidIdentity)?;
        let provider_id = kakao_id(&profile.id).ok_or(OAuthProviderError::InvalidIdentity)?;
        let provider_profile = profile
            .kakao_account
            .and_then(|account| account.profile)
            .unwrap_or_default();
        let fallback = format!("카카오{}", provider_id.chars().take(6).collect::<String>());
        Ok(ProviderIdentity {
            provider_id,
            nickname: bounded_nonempty(provider_profile.nickname, fallback, 64),
            avatar_url: bounded_optional(provider_profile.profile_image_url, 512),
        })
    }
}

impl OAuthProvider for KakaoOAuthProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Kakao
    }

    fn authorization_url(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<String, OAuthProviderError> {
        build_authorization_url(
            KAKAO_AUTHORIZE_URL,
            &[
                ("client_id", self.config.client_id.as_str()),
                ("redirect_uri", request.redirect_uri.as_str()),
                ("response_type", "code"),
                ("state", request.state.as_str()),
                ("code_challenge", request.code_challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("scope", "profile_nickname profile_image"),
            ],
        )
    }

    fn exchange<'a>(
        &'a self,
        request: &'a ProviderExchangeRequest,
    ) -> OAuthProviderFuture<'a, ProviderIdentity> {
        Box::pin(self.exchange_identity(request))
    }
}

pub struct GoogleOAuthProvider {
    config: OAuthClientConfig,
    client: Client,
    id_tokens: GoogleIdTokenVerifier,
}

impl GoogleOAuthProvider {
    pub fn new(config: OAuthClientConfig) -> Result<Self, OAuthProviderError> {
        let client = oauth_client(config.timeout)?;
        let id_tokens = GoogleIdTokenVerifier::new(config.client_id.clone())?;
        Ok(Self {
            config,
            client,
            id_tokens,
        })
    }

    async fn exchange_identity(
        &self,
        request: &ProviderExchangeRequest,
    ) -> Result<ProviderIdentity, OAuthProviderError> {
        let token = self
            .client
            .post(GOOGLE_TOKEN_URL)
            .form(&GoogleTokenRequest {
                grant_type: "authorization_code",
                client_id: &self.config.client_id,
                client_secret: &self.config.client_secret,
                redirect_uri: &request.redirect_uri,
                code: &request.authorization_code,
                code_verifier: &request.code_verifier,
            })
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| provider_unavailable("google_token"))?
            .json::<GoogleToken>()
            .await
            .map_err(|_| provider_unavailable("google_token_decode"))?;
        let subject = self
            .verify_google_id_token(&token.id_token, &request.nonce)
            .await?;
        let profile = self
            .client
            .get(GOOGLE_IDENTITY_URL)
            .bearer_auth(&token.access_token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| provider_unavailable("google_identity"))?
            .json::<GoogleIdentity>()
            .await
            .map_err(|_| OAuthProviderError::InvalidIdentity)?;
        if profile.sub.is_empty() || profile.sub != subject {
            return Err(OAuthProviderError::InvalidIdentity);
        }
        let fallback = format!("Google{}", profile.sub.chars().take(6).collect::<String>());
        Ok(ProviderIdentity {
            provider_id: profile.sub,
            nickname: bounded_nonempty(profile.name, fallback, 64),
            avatar_url: bounded_optional(profile.picture, 512),
        })
    }

    async fn verify_google_id_token(
        &self,
        token: &str,
        expected_nonce: &str,
    ) -> Result<String, OAuthProviderError> {
        let jwks = self
            .client
            .get(GOOGLE_JWKS_URL)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| provider_unavailable("google_jwks"))?
            .json::<JwkSet>()
            .await
            .map_err(|_| provider_unavailable("google_jwks_decode"))?;
        self.id_tokens.verify_subject(token, expected_nonce, &jwks)
    }
}

#[derive(Clone)]
pub struct GoogleIdTokenVerifier {
    client_id: String,
}

impl GoogleIdTokenVerifier {
    pub fn new(client_id: impl Into<String>) -> Result<Self, OAuthProviderError> {
        let client_id = client_id.into();
        if client_id.trim().is_empty() {
            return Err(OAuthProviderError::InvalidConfiguration);
        }
        Ok(Self { client_id })
    }

    pub fn verify_subject(
        &self,
        token: &str,
        expected_nonce: &str,
        jwks: &JwkSet,
    ) -> Result<String, OAuthProviderError> {
        let header = decode_header(token).map_err(|_| OAuthProviderError::InvalidIdentity)?;
        if header.alg != Algorithm::RS256 {
            return Err(OAuthProviderError::InvalidIdentity);
        }
        let key_id = header.kid.ok_or(OAuthProviderError::InvalidIdentity)?;
        let jwk = jwks
            .find(&key_id)
            .ok_or(OAuthProviderError::InvalidIdentity)?;
        let decoding_key =
            DecodingKey::from_jwk(jwk).map_err(|_| OAuthProviderError::InvalidIdentity)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[self.client_id.as_str()]);
        validation.set_issuer(&[GOOGLE_ISSUER]);
        validation.set_required_spec_claims(&["sub", "iss", "aud", "iat", "exp"]);
        let claims = decode::<GoogleIdClaims>(token, &decoding_key, &validation)
            .map_err(|_| OAuthProviderError::InvalidIdentity)?
            .claims;
        if claims.sub.is_empty() || claims.nonce.as_deref() != Some(expected_nonce) {
            return Err(OAuthProviderError::InvalidIdentity);
        }
        Ok(claims.sub)
    }
}

impl OAuthProvider for GoogleOAuthProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Google
    }

    fn authorization_url(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<String, OAuthProviderError> {
        build_authorization_url(
            GOOGLE_AUTHORIZE_URL,
            &[
                ("client_id", self.config.client_id.as_str()),
                ("redirect_uri", request.redirect_uri.as_str()),
                ("response_type", "code"),
                ("scope", "openid profile"),
                ("state", request.state.as_str()),
                ("nonce", request.nonce.as_str()),
                ("code_challenge", request.code_challenge.as_str()),
                ("code_challenge_method", "S256"),
            ],
        )
    }

    fn exchange<'a>(
        &'a self,
        request: &'a ProviderExchangeRequest,
    ) -> OAuthProviderFuture<'a, ProviderIdentity> {
        Box::pin(self.exchange_identity(request))
    }
}

fn oauth_client(timeout: Duration) -> Result<Client, OAuthProviderError> {
    Client::builder()
        .redirect(Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|_| OAuthProviderError::InvalidConfiguration)
}

fn build_authorization_url(
    base: &'static str,
    parameters: &[(&str, &str)],
) -> Result<String, OAuthProviderError> {
    let mut url = Url::parse(base).map_err(|_| OAuthProviderError::InvalidConfiguration)?;
    url.query_pairs_mut()
        .extend_pairs(parameters.iter().copied());
    Ok(url.to_string())
}

fn provider_unavailable(operation: &'static str) -> OAuthProviderError {
    tracing::warn!(
        dependency = "oauth_provider",
        failure_kind = "upstream",
        operation,
        "OAuth provider operation failed"
    );
    OAuthProviderError::Unavailable
}

fn kakao_id(value: &serde_json::Value) -> Option<String> {
    value.as_u64().map(|value| value.to_string()).or_else(|| {
        value
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn bounded_nonempty(value: Option<String>, fallback: String, maximum: usize) -> String {
    let value = value.filter(|value| !value.is_empty()).unwrap_or(fallback);
    value.chars().take(maximum).collect()
}

fn bounded_optional(value: Option<String>, maximum: usize) -> Option<String> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(maximum).collect())
}

#[derive(serde::Serialize)]
struct KakaoTokenRequest<'a> {
    grant_type: &'static str,
    client_id: &'a str,
    client_secret: &'a str,
    redirect_uri: &'a str,
    code: &'a str,
    code_verifier: &'a str,
}

#[derive(serde::Serialize)]
struct GoogleTokenRequest<'a> {
    grant_type: &'static str,
    client_id: &'a str,
    client_secret: &'a str,
    redirect_uri: &'a str,
    code: &'a str,
    code_verifier: &'a str,
}

#[derive(Deserialize)]
struct OAuthAccessToken {
    access_token: String,
}

#[derive(Deserialize)]
struct GoogleToken {
    access_token: String,
    id_token: String,
}

#[derive(Deserialize)]
struct KakaoIdentity {
    id: serde_json::Value,
    kakao_account: Option<KakaoAccount>,
}

#[derive(Deserialize)]
struct KakaoAccount {
    profile: Option<KakaoProfile>,
}

#[derive(Default, Deserialize)]
struct KakaoProfile {
    nickname: Option<String>,
    profile_image_url: Option<String>,
}

#[derive(Deserialize)]
struct GoogleIdentity {
    sub: String,
    name: Option<String>,
    picture: Option<String>,
}

#[derive(Deserialize)]
struct GoogleIdClaims {
    sub: String,
    nonce: Option<String>,
    #[allow(dead_code)]
    iss: String,
    #[allow(dead_code)]
    aud: serde_json::Value,
    #[allow(dead_code)]
    iat: u64,
    #[allow(dead_code)]
    exp: u64,
    #[serde(flatten)]
    _additional: HashMap<String, serde_json::Value>,
}
