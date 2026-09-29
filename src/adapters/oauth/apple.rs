//! Sign in with Apple identity-token verification and revocation adapter.

use std::{collections::HashMap, time::Duration};

use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
    jwk::JwkSet,
};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::{
    config::auth::{APPLE_ISSUER, APPLE_JWKS_URL, APPLE_REVOKE_URL, APPLE_TOKEN_URL},
    ports::apple_identity_provider::{
        AppleAuthorizationCodeRevocationRequest, AppleIdentity, AppleIdentityProvider,
        AppleIdentityProviderError, AppleIdentityProviderFuture, AppleIdentityVerificationRequest,
        AppleRevocationProvider, AppleRevocationProviderError, AppleRevocationProviderFuture,
    },
};

const APPLE_CLOCK_LEEWAY_SECONDS: u64 = 300;
const APPLE_CLIENT_SECRET_TTL_SECONDS: i64 = 300;

#[derive(Clone)]
pub struct AppleOAuthConfig {
    audiences: Vec<String>,
    team_id: String,
    key_id: String,
    private_key_der: Vec<u8>,
    timeout: Duration,
}

impl AppleOAuthConfig {
    pub fn new(
        audiences: Vec<String>,
        team_id: impl Into<String>,
        key_id: impl Into<String>,
        private_key_der: Vec<u8>,
        timeout: Duration,
    ) -> Result<Self, AppleIdentityProviderError> {
        let team_id = team_id.into();
        let key_id = key_id.into();
        if audiences.is_empty()
            || audiences.iter().any(|audience| audience.trim().is_empty())
            || team_id.trim().is_empty()
            || key_id.trim().is_empty()
            || private_key_der.is_empty()
            || timeout.is_zero()
            || timeout > Duration::from_secs(30)
        {
            return Err(AppleIdentityProviderError::InvalidConfiguration);
        }
        Ok(Self {
            audiences,
            team_id,
            key_id,
            private_key_der,
            timeout,
        })
    }
}

pub struct AppleOAuthProvider {
    config: AppleOAuthConfig,
    client: Client,
    endpoints: AppleOAuthEndpoints,
    id_tokens: AppleIdTokenVerifier,
}

#[derive(Clone)]
struct AppleOAuthEndpoints {
    keys_endpoint: String,
    token_endpoint: String,
    revoke_endpoint: String,
}

impl AppleOAuthEndpoints {
    fn production() -> Self {
        Self {
            keys_endpoint: APPLE_JWKS_URL.to_owned(),
            token_endpoint: APPLE_TOKEN_URL.to_owned(),
            revoke_endpoint: APPLE_REVOKE_URL.to_owned(),
        }
    }
}

impl AppleOAuthProvider {
    pub fn new(config: AppleOAuthConfig) -> Result<Self, AppleIdentityProviderError> {
        Self::with_endpoints(config, AppleOAuthEndpoints::production())
    }

    #[cfg(test)]
    fn new_with_endpoints(
        config: AppleOAuthConfig,
        keys_endpoint: String,
        token_endpoint: String,
        revoke_endpoint: String,
    ) -> Result<Self, AppleIdentityProviderError> {
        Self::with_endpoints(
            config,
            AppleOAuthEndpoints {
                keys_endpoint,
                token_endpoint,
                revoke_endpoint,
            },
        )
    }

    fn with_endpoints(
        config: AppleOAuthConfig,
        endpoints: AppleOAuthEndpoints,
    ) -> Result<Self, AppleIdentityProviderError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(config.timeout)
            .build()
            .map_err(|_| AppleIdentityProviderError::InvalidConfiguration)?;
        let id_tokens = AppleIdTokenVerifier::new(config.audiences.clone())?;
        Ok(Self {
            config,
            client,
            endpoints,
            id_tokens,
        })
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, AppleIdentityProviderError> {
        self.client
            .get(&self.endpoints.keys_endpoint)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| apple_identity_unavailable("apple_jwks"))?
            .json::<JwkSet>()
            .await
            .map_err(|_| apple_identity_unavailable("apple_jwks_decode"))
    }

    async fn revoke_code(
        &self,
        request: &AppleAuthorizationCodeRevocationRequest,
    ) -> Result<(), AppleRevocationProviderError> {
        let client_secret = self.client_secret(&request.client_id)?;
        let token = self
            .exchange_code(
                &request.client_id,
                &request.authorization_code,
                &client_secret,
            )
            .await?;
        let (token, hint) = match token.refresh_token {
            Some(refresh_token) => (refresh_token, "refresh_token"),
            None => (token.access_token, "access_token"),
        };
        let response = self
            .client
            .post(&self.endpoints.revoke_endpoint)
            .form(&AppleRevokeRequest {
                client_id: &request.client_id,
                client_secret: &client_secret,
                token: &token,
                token_type_hint: hint,
            })
            .send()
            .await
            .map_err(|_| apple_revocation_unavailable("apple_revoke"))?;
        match response.status() {
            status if status.is_success() => Ok(()),
            status if status.is_server_error() => Err(apple_revocation_unavailable("apple_revoke")),
            _ => Err(AppleRevocationProviderError::InvalidAuthorizationCode),
        }
    }

    async fn exchange_code(
        &self,
        client_id: &str,
        authorization_code: &str,
        client_secret: &str,
    ) -> Result<AppleTokenResponse, AppleRevocationProviderError> {
        let response = self
            .client
            .post(&self.endpoints.token_endpoint)
            .form(&AppleTokenRequest {
                grant_type: "authorization_code",
                client_id,
                client_secret,
                code: authorization_code,
            })
            .send()
            .await
            .map_err(|_| apple_revocation_unavailable("apple_token"))?;
        match response.status() {
            status if status.is_success() => response
                .json::<AppleTokenResponse>()
                .await
                .map_err(|_| apple_revocation_unavailable("apple_token_decode")),
            status if status.is_server_error() => Err(apple_revocation_unavailable("apple_token")),
            StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(AppleRevocationProviderError::InvalidAuthorizationCode)
            }
            _ => Err(AppleRevocationProviderError::InvalidAuthorizationCode),
        }
    }

    fn client_secret(&self, client_id: &str) -> Result<String, AppleRevocationProviderError> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let exp = now
            .checked_add(APPLE_CLIENT_SECRET_TTL_SECONDS)
            .ok_or(AppleRevocationProviderError::InvalidConfiguration)?;
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());
        let claims = AppleClientSecretClaims {
            iss: &self.config.team_id,
            sub: client_id,
            aud: APPLE_ISSUER,
            iat: now,
            exp,
        };
        let key = EncodingKey::from_ec_der(&self.config.private_key_der);
        encode(&header, &claims, &key)
            .map_err(|_| AppleRevocationProviderError::InvalidConfiguration)
    }
}

impl AppleIdentityProvider for AppleOAuthProvider {
    fn verify_identity<'a>(
        &'a self,
        request: &'a AppleIdentityVerificationRequest,
    ) -> AppleIdentityProviderFuture<'a, AppleIdentity> {
        Box::pin(async move {
            let jwks = self.fetch_jwks().await?;
            self.id_tokens
                .verify_identity(&request.identity_token, &request.raw_nonce, &jwks)
        })
    }
}

impl AppleRevocationProvider for AppleOAuthProvider {
    fn revoke_authorization_code<'a>(
        &'a self,
        request: &'a AppleAuthorizationCodeRevocationRequest,
    ) -> AppleRevocationProviderFuture<'a> {
        Box::pin(self.revoke_code(request))
    }
}

#[derive(Clone)]
pub struct AppleIdTokenVerifier {
    audiences: Vec<String>,
}

impl AppleIdTokenVerifier {
    pub fn new(audiences: Vec<String>) -> Result<Self, AppleIdentityProviderError> {
        if audiences.is_empty() || audiences.iter().any(|audience| audience.trim().is_empty()) {
            return Err(AppleIdentityProviderError::InvalidConfiguration);
        }
        Ok(Self { audiences })
    }

    pub fn verify_identity(
        &self,
        token: &str,
        raw_nonce: &str,
        jwks: &JwkSet,
    ) -> Result<AppleIdentity, AppleIdentityProviderError> {
        let header =
            decode_header(token).map_err(|_| apple_invalid_identity("malformed_header"))?;
        if header.alg != Algorithm::RS256 {
            return Err(apple_invalid_identity("unexpected_algorithm"));
        }
        let key_id = header
            .kid
            .ok_or_else(|| apple_invalid_identity("missing_kid"))?;
        let jwk = jwks
            .find(&key_id)
            .ok_or_else(|| apple_invalid_identity("kid_not_found"))?;
        let decoding_key =
            DecodingKey::from_jwk(jwk).map_err(|_| apple_invalid_identity("invalid_jwk"))?;
        let mut validation = Validation::new(Algorithm::RS256);
        let audience_refs = self
            .audiences
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        validation.set_audience(&audience_refs);
        validation.set_issuer(&[APPLE_ISSUER]);
        validation.set_required_spec_claims(&["sub", "iss", "aud", "iat", "exp"]);
        validation.leeway = APPLE_CLOCK_LEEWAY_SECONDS;
        let claims = decode::<AppleIdClaims>(token, &decoding_key, &validation)
            .map_err(|_| apple_invalid_identity("claims_rejected"))?
            .claims;
        if claims.sub.is_empty() || claims.sub.len() > 128 {
            return Err(apple_invalid_identity("subject_invalid"));
        }
        let expected_nonce = sha256_hex(raw_nonce.as_bytes());
        if claims.nonce.as_deref() != Some(expected_nonce.as_str()) {
            return Err(apple_invalid_identity("nonce_mismatch"));
        }
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let iat = i64::try_from(claims.iat).map_err(|_| apple_invalid_identity("iat_invalid"))?;
        let leeway = i64::try_from(APPLE_CLOCK_LEEWAY_SECONDS)
            .map_err(|_| apple_invalid_identity("iat_invalid"))?;
        if iat > now.saturating_add(leeway) {
            return Err(apple_invalid_identity("iat_in_future"));
        }
        if !self
            .audiences
            .iter()
            .any(|audience| audience == &claims.aud)
        {
            return Err(apple_invalid_identity("audience_not_allowed"));
        }
        Ok(AppleIdentity {
            provider_id: claims.sub,
            client_id: claims.aud,
        })
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn apple_invalid_identity(reason: &'static str) -> AppleIdentityProviderError {
    tracing::warn!(
        dependency = "apple",
        failure_kind = "identity_token",
        reason,
        "Apple identity token verification failed"
    );
    AppleIdentityProviderError::InvalidIdentity
}

fn apple_identity_unavailable(operation: &'static str) -> AppleIdentityProviderError {
    tracing::warn!(
        dependency = "apple",
        failure_kind = "upstream",
        operation,
        "Apple identity provider operation failed"
    );
    AppleIdentityProviderError::Unavailable
}

fn apple_revocation_unavailable(operation: &'static str) -> AppleRevocationProviderError {
    tracing::warn!(
        dependency = "apple",
        failure_kind = "upstream",
        operation,
        "Apple revocation provider operation failed"
    );
    AppleRevocationProviderError::Unavailable
}

#[derive(Serialize)]
struct AppleClientSecretClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

#[derive(Serialize)]
struct AppleTokenRequest<'a> {
    grant_type: &'static str,
    client_id: &'a str,
    client_secret: &'a str,
    code: &'a str,
}

#[derive(Serialize)]
struct AppleRevokeRequest<'a> {
    client_id: &'a str,
    client_secret: &'a str,
    token: &'a str,
    token_type_hint: &'a str,
}

#[derive(Deserialize)]
struct AppleTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Clone, Deserialize)]
struct AppleIdClaims {
    sub: String,
    aud: String,
    nonce: Option<String>,
    iat: u64,
    #[allow(dead_code)]
    iss: String,
    #[allow(dead_code)]
    exp: u64,
    #[serde(flatten)]
    _additional: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        error::Error,
        io,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use aws_lc_rs::{
        encoding::AsDer,
        rand::SystemRandom,
        rsa::KeySize,
        signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, RsaKeyPair},
    };
    use axum::{
        Json, Router,
        body::{Body, to_bytes},
        extract::State,
        http::{Request, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use jsonwebtoken::{DecodingKey, EncodingKey, Header, decode, encode};
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::oneshot};
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;
    use crate::platform::logging::build_json_subscriber;

    type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

    #[test]
    fn apple_id_token_verifier_accepts_signed_token_and_collapses_validation_failures() -> TestResult
    {
        let rsa = RsaKeyPair::generate(KeySize::Rsa2048)?;
        let private_der = rsa.as_der()?;
        let jwks = jwks_for_rsa(&rsa, "apple-key-1")?;
        let verifier = AppleIdTokenVerifier::new(vec!["dev.local.jamyeapp".to_owned()])?;
        let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp())?;
        let raw_nonce = "raw-nonce-123456";
        let claims = test_apple_claims(
            "apple-subject-1",
            "dev.local.jamyeapp",
            raw_nonce,
            now,
            now + 600,
            APPLE_ISSUER,
        );
        let token = signed_rsa_token(private_der.as_ref(), "apple-key-1", &claims)?;
        let other_rsa = RsaKeyPair::generate(KeySize::Rsa2048)?;
        let other_private_der = other_rsa.as_der()?;

        let identity = verifier.verify_identity(&token, raw_nonce, &jwks)?;
        assert_eq!(identity.provider_id, "apple-subject-1");
        assert_eq!(identity.client_id, "dev.local.jamyeapp");

        for (label, token, nonce) in [
            (
                "signature",
                signed_rsa_token(other_private_der.as_ref(), "apple-key-1", &claims)?,
                raw_nonce,
            ),
            (
                "kid",
                signed_rsa_token(private_der.as_ref(), "other-key", &claims)?,
                raw_nonce,
            ),
            (
                "iss",
                signed_rsa_token(
                    private_der.as_ref(),
                    "apple-key-1",
                    &test_apple_claims(
                        "apple-subject-1",
                        "dev.local.jamyeapp",
                        raw_nonce,
                        now,
                        now + 600,
                        "https://issuer.example",
                    ),
                )?,
                raw_nonce,
            ),
            (
                "aud",
                signed_rsa_token(
                    private_der.as_ref(),
                    "apple-key-1",
                    &test_apple_claims(
                        "apple-subject-1",
                        "other.bundle",
                        raw_nonce,
                        now,
                        now + 600,
                        APPLE_ISSUER,
                    ),
                )?,
                raw_nonce,
            ),
            (
                "exp",
                signed_rsa_token(
                    private_der.as_ref(),
                    "apple-key-1",
                    &test_apple_claims(
                        "apple-subject-1",
                        "dev.local.jamyeapp",
                        raw_nonce,
                        now.saturating_sub(1_000),
                        now.saturating_sub(900),
                        APPLE_ISSUER,
                    ),
                )?,
                raw_nonce,
            ),
            ("nonce", token.clone(), "other-raw-nonce-123456"),
            (
                "sub",
                signed_rsa_token(
                    private_der.as_ref(),
                    "apple-key-1",
                    &test_apple_claims(
                        "",
                        "dev.local.jamyeapp",
                        raw_nonce,
                        now,
                        now + 600,
                        APPLE_ISSUER,
                    ),
                )?,
                raw_nonce,
            ),
            ("alg", signed_hs_token("apple-key-1", &claims)?, raw_nonce),
        ] {
            assert_eq!(
                verifier.verify_identity(&token, nonce, &jwks),
                Err(AppleIdentityProviderError::InvalidIdentity),
                "{label} failure did not collapse to apple_identity_token_invalid"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn apple_provider_uses_fake_endpoints_and_verifiable_client_secret() -> TestResult {
        let rsa = RsaKeyPair::generate(KeySize::Rsa2048)?;
        let rsa_private_der = rsa.as_der()?;
        let jwks = jwks_for_rsa(&rsa, "apple-key-1")?;
        let ec = generated_ec_key_material()?;
        let captured = CapturedAppleRequests::default();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let app = Router::new()
            .route("/keys", get(apple_keys))
            .route("/auth/token", post(apple_token))
            .route("/auth/revoke", post(apple_revoke))
            .with_state(FakeAppleState {
                jwks: jwks_json(&jwks)?,
                captured: captured.clone(),
            });
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
        });

        let provider = AppleOAuthProvider::new_with_endpoints(
            AppleOAuthConfig::new(
                vec!["dev.local.jamyeapp".to_owned()],
                "TEAMID123",
                "KEYID123",
                ec.private_der.clone(),
                Duration::from_secs(2),
            )?,
            format!("http://{address}/keys"),
            format!("http://{address}/auth/token"),
            format!("http://{address}/auth/revoke"),
        )?;
        let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp())?;
        let raw_nonce = "raw-nonce-provider";
        let identity_token = signed_rsa_token(
            rsa_private_der.as_ref(),
            "apple-key-1",
            &test_apple_claims(
                "apple-provider-subject",
                "dev.local.jamyeapp",
                raw_nonce,
                now,
                now + 600,
                APPLE_ISSUER,
            ),
        )?;

        let identity = provider
            .verify_identity(&AppleIdentityVerificationRequest {
                identity_token,
                raw_nonce: raw_nonce.to_owned(),
            })
            .await?;
        assert_eq!(identity.provider_id, "apple-provider-subject");
        assert_eq!(identity.client_id, "dev.local.jamyeapp");
        provider
            .revoke_authorization_code(&AppleAuthorizationCodeRevocationRequest {
                client_id: "dev.local.jamyeapp".to_owned(),
                authorization_code: "APPLE_AUTHORIZATION_CODE_SENTINEL".to_owned(),
            })
            .await?;

        let requests = captured.requests()?;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/auth/token");
        assert_eq!(
            requests[0].fields.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(
            requests[0].fields.get("client_id").map(String::as_str),
            Some("dev.local.jamyeapp")
        );
        assert_eq!(
            requests[0].fields.get("code").map(String::as_str),
            Some("APPLE_AUTHORIZATION_CODE_SENTINEL")
        );
        let token_claims = decode_client_secret(
            requests[0]
                .fields
                .get("client_secret")
                .ok_or_else(|| io::Error::other("missing token client_secret"))?,
            &ec.public_x,
            &ec.public_y,
        )?;
        assert_eq!(token_claims.iss, "TEAMID123");
        assert_eq!(token_claims.sub, "dev.local.jamyeapp");
        assert_eq!(token_claims.aud, APPLE_ISSUER);
        assert!(token_claims.exp > token_claims.iat);

        assert_eq!(requests[1].path, "/auth/revoke");
        assert_eq!(
            requests[1].fields.get("token").map(String::as_str),
            Some("APPLE_REFRESH_TOKEN_SENTINEL")
        );
        assert_eq!(
            requests[1]
                .fields
                .get("token_type_hint")
                .map(String::as_str),
            Some("refresh_token")
        );
        let revoke_claims = decode_client_secret(
            requests[1]
                .fields
                .get("client_secret")
                .ok_or_else(|| io::Error::other("missing revoke client_secret"))?,
            &ec.public_x,
            &ec.public_y,
        )?;
        assert_eq!(revoke_claims.sub, "dev.local.jamyeapp");

        let _ = shutdown_tx.send(());
        server.await??;
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apple_provider_failure_logs_exclude_credentials_and_client_secret() -> TestResult {
        let writer = SharedWriter::default();
        let output = writer.clone();
        let subscriber = build_json_subscriber(writer, "info")?;
        let _guard = tracing::subscriber::set_default(subscriber);
        let rsa = RsaKeyPair::generate(KeySize::Rsa2048)?;
        let rsa_private_der = rsa.as_der()?;
        let jwks = jwks_for_rsa(&rsa, "apple-key-1")?;
        let ec = generated_ec_key_material()?;
        let captured = CapturedAppleRequests::default();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let app = Router::new()
            .route("/keys", get(apple_keys))
            .route("/auth/token", post(apple_token_unavailable))
            .route("/auth/revoke", post(apple_revoke))
            .with_state(FakeAppleState {
                jwks: jwks_json(&jwks)?,
                captured: captured.clone(),
            });
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
        });
        let provider = AppleOAuthProvider::new_with_endpoints(
            AppleOAuthConfig::new(
                vec!["dev.local.jamyeapp".to_owned()],
                "TEAMID123",
                "KEYID123",
                ec.private_der,
                Duration::from_secs(2),
            )?,
            format!("http://{address}/keys"),
            format!("http://{address}/auth/token"),
            format!("http://{address}/auth/revoke"),
        )?;
        let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp())?;
        let signed_raw_nonce = "raw-nonce-signed-provider";
        let logged_raw_nonce = "APPLE_ADAPTER_RAW_NONCE_SENTINEL";
        let identity_token = signed_rsa_token(
            rsa_private_der.as_ref(),
            "apple-key-1",
            &test_apple_claims(
                "apple-provider-log-subject",
                "dev.local.jamyeapp",
                signed_raw_nonce,
                now,
                now + 600,
                APPLE_ISSUER,
            ),
        )?;

        assert_eq!(
            provider
                .verify_identity(&AppleIdentityVerificationRequest {
                    identity_token: identity_token.clone(),
                    raw_nonce: logged_raw_nonce.to_owned(),
                })
                .await,
            Err(AppleIdentityProviderError::InvalidIdentity)
        );
        assert_eq!(
            provider
                .revoke_authorization_code(&AppleAuthorizationCodeRevocationRequest {
                    client_id: "dev.local.jamyeapp".to_owned(),
                    authorization_code: "APPLE_ADAPTER_CODE_SENTINEL".to_owned(),
                })
                .await,
            Err(AppleRevocationProviderError::Unavailable)
        );

        let requests = captured.requests()?;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/auth/token");
        let client_secret = requests[0]
            .fields
            .get("client_secret")
            .ok_or_else(|| io::Error::other("missing token client_secret"))?;
        let logs = output.snapshot()?;
        assert!(logs.contains("Apple identity token verification failed"));
        assert!(logs.contains("Apple revocation provider operation failed"));
        for forbidden in [
            identity_token.as_str(),
            logged_raw_nonce,
            "APPLE_ADAPTER_CODE_SENTINEL",
            client_secret.as_str(),
        ] {
            assert!(
                !logs.contains(forbidden),
                "Apple adapter logs leaked credential material"
            );
        }

        let _ = shutdown_tx.send(());
        server.await??;
        Ok(())
    }

    #[derive(Clone, Serialize)]
    struct TestAppleClaims {
        sub: String,
        aud: String,
        nonce: String,
        iat: u64,
        exp: u64,
        iss: String,
    }

    fn test_apple_claims(
        sub: &str,
        aud: &str,
        raw_nonce: &str,
        iat: u64,
        exp: u64,
        iss: &str,
    ) -> TestAppleClaims {
        TestAppleClaims {
            sub: sub.to_owned(),
            aud: aud.to_owned(),
            nonce: sha256_hex(raw_nonce.as_bytes()),
            iat,
            exp,
            iss: iss.to_owned(),
        }
    }

    fn signed_rsa_token(
        private_der: &[u8],
        kid: &str,
        claims: &TestAppleClaims,
    ) -> TestResult<String> {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_owned());
        // aws-lc-rs exports generated RSA keys as PKCS#8, while
        // `EncodingKey::from_rsa_der` takes the PKCS#1 `RSAPrivateKey` nested
        // inside that document.
        let pkcs1 = pkcs8_inner_private_key(private_der)?;
        Ok(encode(&header, claims, &EncodingKey::from_rsa_der(&pkcs1))?)
    }

    fn pkcs8_inner_private_key(der: &[u8]) -> TestResult<Vec<u8>> {
        let mut index = 0;
        read_tag(der, &mut index, 0x30)?;
        read_len(der, &mut index)?;
        // The PKCS#8 version is INTEGER 0; skip it without `read_integer`,
        // which rejects a value that is empty after stripping leading zeros.
        read_tag(der, &mut index, 0x02)?;
        let version_len = read_len(der, &mut index)?;
        index = index
            .checked_add(version_len)
            .ok_or_else(|| io::Error::other("PKCS#8 version length overflow"))?;
        read_tag(der, &mut index, 0x30)?;
        let algorithm_len = read_len(der, &mut index)?;
        index = index
            .checked_add(algorithm_len)
            .ok_or_else(|| io::Error::other("PKCS#8 algorithm length overflow"))?;
        read_tag(der, &mut index, 0x04)?;
        let key_len = read_len(der, &mut index)?;
        let end = index
            .checked_add(key_len)
            .ok_or_else(|| io::Error::other("PKCS#8 private key length overflow"))?;
        Ok(der
            .get(index..end)
            .ok_or_else(|| io::Error::other("PKCS#8 private key truncated"))?
            .to_vec())
    }

    fn signed_hs_token(
        kid: &str,
        claims: &TestAppleClaims,
    ) -> Result<String, jsonwebtoken::errors::Error> {
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some(kid.to_owned());
        encode(&header, claims, &EncodingKey::from_secret(b"not-apple"))
    }

    fn jwks_for_rsa(rsa: &RsaKeyPair, kid: &str) -> TestResult<JwkSet> {
        let (modulus, exponent) = rsa_public_components(rsa.public_key().as_ref())?;
        Ok(serde_json::from_value(json!({
            "keys": [{
                "kty": "RSA",
                "kid": kid,
                "alg": "RS256",
                "use": "sig",
                "n": URL_SAFE_NO_PAD.encode(modulus),
                "e": URL_SAFE_NO_PAD.encode(exponent),
            }]
        }))?)
    }

    fn jwks_json(jwks: &JwkSet) -> TestResult<Value> {
        Ok(serde_json::to_value(jwks)?)
    }

    fn rsa_public_components(der: &[u8]) -> TestResult<(Vec<u8>, Vec<u8>)> {
        let mut index = 0;
        read_tag(der, &mut index, 0x30)?;
        let sequence_len = read_len(der, &mut index)?;
        let sequence_end = index
            .checked_add(sequence_len)
            .ok_or_else(|| io::Error::other("RSA sequence length overflow"))?;
        if sequence_end != der.len() {
            return Err(io::Error::other("RSA public key has trailing data").into());
        }
        let modulus = read_integer(der, &mut index)?;
        let exponent = read_integer(der, &mut index)?;
        if index != sequence_end {
            return Err(io::Error::other("RSA public key sequence not fully consumed").into());
        }
        Ok((modulus, exponent))
    }

    fn read_integer(der: &[u8], index: &mut usize) -> TestResult<Vec<u8>> {
        read_tag(der, index, 0x02)?;
        let len = read_len(der, index)?;
        let end = index
            .checked_add(len)
            .ok_or_else(|| io::Error::other("ASN.1 integer length overflow"))?;
        let value = der
            .get(*index..end)
            .ok_or_else(|| io::Error::other("ASN.1 integer truncated"))?;
        *index = end;
        let trimmed = value
            .iter()
            .skip_while(|byte| **byte == 0)
            .copied()
            .collect::<Vec<_>>();
        if trimmed.is_empty() {
            return Err(io::Error::other("ASN.1 integer is empty").into());
        }
        Ok(trimmed)
    }

    fn read_tag(der: &[u8], index: &mut usize, tag: u8) -> TestResult {
        let actual = *der
            .get(*index)
            .ok_or_else(|| io::Error::other("ASN.1 tag missing"))?;
        if actual != tag {
            return Err(io::Error::other("ASN.1 tag mismatch").into());
        }
        *index += 1;
        Ok(())
    }

    fn read_len(der: &[u8], index: &mut usize) -> TestResult<usize> {
        let first = *der
            .get(*index)
            .ok_or_else(|| io::Error::other("ASN.1 length missing"))?;
        *index += 1;
        if first & 0x80 == 0 {
            return Ok(usize::from(first));
        }
        let count = usize::from(first & 0x7f);
        if count == 0 || count > std::mem::size_of::<usize>() {
            return Err(io::Error::other("ASN.1 length is unsupported").into());
        }
        let mut length = 0_usize;
        for _ in 0..count {
            let byte = *der
                .get(*index)
                .ok_or_else(|| io::Error::other("ASN.1 length truncated"))?;
            *index += 1;
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(byte)))
                .ok_or_else(|| io::Error::other("ASN.1 length overflow"))?;
        }
        Ok(length)
    }

    struct EcKeyMaterial {
        private_der: Vec<u8>,
        public_x: String,
        public_y: String,
    }

    fn generated_ec_key_material() -> TestResult<EcKeyMaterial> {
        let rng = SystemRandom::new();
        let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)?;
        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref())?;
        let public = key_pair.public_key().as_ref();
        if public.len() != 65 || public.first().copied() != Some(0x04) {
            return Err(io::Error::other("unexpected P-256 public key shape").into());
        }
        Ok(EcKeyMaterial {
            private_der: document.as_ref().to_vec(),
            public_x: URL_SAFE_NO_PAD.encode(&public[1..33]),
            public_y: URL_SAFE_NO_PAD.encode(&public[33..65]),
        })
    }

    #[derive(Clone, Default)]
    struct CapturedAppleRequests(Arc<Mutex<Vec<CapturedAppleRequest>>>);

    impl CapturedAppleRequests {
        fn requests(&self) -> TestResult<Vec<CapturedAppleRequest>> {
            Ok(self
                .0
                .lock()
                .map_err(|_| io::Error::other("captured Apple request lock poisoned"))?
                .clone())
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct CapturedAppleRequest {
        path: String,
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone)]
    struct FakeAppleState {
        jwks: Value,
        captured: CapturedAppleRequests,
    }

    async fn apple_keys(State(state): State<FakeAppleState>) -> Json<Value> {
        Json(state.jwks)
    }

    async fn apple_token(
        State(state): State<FakeAppleState>,
        request: Request<Body>,
    ) -> impl IntoResponse {
        match capture_form(&state.captured, request).await {
            Ok(()) => (
                StatusCode::OK,
                Json(json!({
                    "access_token": "APPLE_ACCESS_TOKEN_SENTINEL",
                    "refresh_token": "APPLE_REFRESH_TOKEN_SENTINEL"
                })),
            )
                .into_response(),
            Err(()) => StatusCode::BAD_REQUEST.into_response(),
        }
    }

    async fn apple_token_unavailable(
        State(state): State<FakeAppleState>,
        request: Request<Body>,
    ) -> impl IntoResponse {
        match capture_form(&state.captured, request).await {
            Ok(()) => StatusCode::INTERNAL_SERVER_ERROR,
            Err(()) => StatusCode::BAD_REQUEST,
        }
    }

    async fn apple_revoke(
        State(state): State<FakeAppleState>,
        request: Request<Body>,
    ) -> impl IntoResponse {
        match capture_form(&state.captured, request).await {
            Ok(()) => StatusCode::OK,
            Err(()) => StatusCode::BAD_REQUEST,
        }
    }

    async fn capture_form(
        captured: &CapturedAppleRequests,
        request: Request<Body>,
    ) -> Result<(), ()> {
        let path = request.uri().path().to_owned();
        let (_, body) = request.into_parts();
        let body = to_bytes(body, 16 * 1024).await.map_err(|_| ())?;
        let fields = url::form_urlencoded::parse(&body)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<BTreeMap<_, _>>();
        captured
            .0
            .lock()
            .map_err(|_| ())?
            .push(CapturedAppleRequest { path, fields });
        Ok(())
    }

    #[derive(Clone, Debug, Deserialize)]
    struct DecodedClientSecret {
        iss: String,
        sub: String,
        aud: String,
        iat: i64,
        exp: i64,
    }

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl SharedWriter {
        fn snapshot(&self) -> io::Result<String> {
            let bytes = self
                .0
                .lock()
                .map_err(|_| io::Error::other("Apple log writer lock poisoned"))?
                .clone();
            String::from_utf8(bytes).map_err(io::Error::other)
        }
    }

    impl<'writer> MakeWriter<'writer> for SharedWriter {
        type Writer = SharedWriterGuard;

        fn make_writer(&'writer self) -> Self::Writer {
            SharedWriterGuard(self.0.clone())
        }
    }

    struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

    impl io::Write for SharedWriterGuard {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| io::Error::other("Apple log writer lock poisoned"))?
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn decode_client_secret(
        token: &str,
        public_x: &str,
        public_y: &str,
    ) -> TestResult<DecodedClientSecret> {
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&[APPLE_ISSUER]);
        validation.set_required_spec_claims(&["iss", "sub", "aud", "iat", "exp"]);
        Ok(decode::<DecodedClientSecret>(
            token,
            &DecodingKey::from_ec_components(public_x, public_y)?,
            &validation,
        )?
        .claims)
    }
}
