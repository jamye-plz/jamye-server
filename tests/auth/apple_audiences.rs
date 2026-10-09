//! Characterization of the multi-audience Sign in with Apple path (task-20a).
//!
//! `JAMYE_APPLE_AUDIENCES` already accepts a comma-separated list. The verifier checks the
//! token `aud` against the whole list and reports the matching audience as `client_id`,
//! which account deletion passes unchanged to the Apple revoke call
//! (`src/application/account_deletion/mod.rs`). This test pins that behavior so the
//! production and development bundle ids can be listed together by environment only.

use std::io;

use aws_lc_rs::{
    encoding::AsDer,
    rand::SystemRandom,
    rsa::KeySize,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, RsaKeyPair},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use jamye_server::{
    adapters::oauth::AppleIdTokenVerifier,
    config::auth::{APPLE_ISSUER, AuthConfig, AuthConfigInput},
    ports::apple_identity_provider::AppleIdentityProviderError,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::JwkSet};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::TestResult;

const PRODUCTION_BUNDLE: &str = "com.ridewithmin.jamyeapp";
const DEVELOPMENT_BUNDLE: &str = "dev.local.jamyeapp";
const KEY_ID: &str = "apple-audience-test-key";
const RAW_NONCE: &str = "raw-nonce-123456";

#[test]
fn apple_audiences_env_list_accepts_either_bundle_id_and_reports_the_matching_client_id()
-> TestResult {
    let ec = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())?;
    let ec_der: &[u8] = ec.as_ref();
    let config = AuthConfig::try_from(AuthConfigInput {
        apple_signin_enabled: Some("true".to_owned()),
        apple_audiences: Some(format!("{PRODUCTION_BUNDLE}, {DEVELOPMENT_BUNDLE}")),
        apple_team_id: Some("team-id".to_owned()),
        apple_key_id: Some("key-id".to_owned()),
        apple_private_key: Some(STANDARD.encode(ec_der)),
        access_token_secret: Some("a-secret-with-at-least-thirty-two-bytes".to_owned()),
        access_token_issuer: Some("https://api.jamye.test".to_owned()),
        access_token_audience: Some("jamye-mobile".to_owned()),
        ..AuthConfigInput::default()
    })?;
    assert_eq!(
        config.apple.audiences,
        vec![PRODUCTION_BUNDLE.to_owned(), DEVELOPMENT_BUNDLE.to_owned()]
    );

    let verifier = AppleIdTokenVerifier::new(config.apple.audiences.clone())?;
    let rsa = RsaKeyPair::generate(KeySize::Rsa2048)?;
    let private_der = rsa.as_der()?;
    let jwks = jwks_for_rsa(&rsa)?;
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp())?;

    for audience in [PRODUCTION_BUNDLE, DEVELOPMENT_BUNDLE] {
        let token = signed_rsa_token(private_der.as_ref(), &claims(audience, now))?;
        let identity = verifier.verify_identity(&token, RAW_NONCE, &jwks)?;
        assert_eq!(identity.provider_id, "apple-subject-1");
        assert_eq!(identity.client_id, audience);
    }

    let foreign = signed_rsa_token(private_der.as_ref(), &claims("other.bundle", now))?;
    assert!(matches!(
        verifier.verify_identity(&foreign, RAW_NONCE, &jwks),
        Err(AppleIdentityProviderError::InvalidIdentity)
    ));
    Ok(())
}

#[derive(Serialize)]
struct Claims {
    sub: String,
    aud: String,
    nonce: String,
    iat: u64,
    exp: u64,
    iss: String,
}

fn claims(audience: &str, now: u64) -> Claims {
    Claims {
        sub: "apple-subject-1".to_owned(),
        aud: audience.to_owned(),
        nonce: sha256_hex(RAW_NONCE.as_bytes()),
        iat: now,
        exp: now + 600,
        iss: APPLE_ISSUER.to_owned(),
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

fn signed_rsa_token(private_der: &[u8], claims: &Claims) -> TestResult<String> {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KEY_ID.to_owned());
    // aws-lc-rs exports generated RSA keys as PKCS#8, while `EncodingKey::from_rsa_der`
    // takes the PKCS#1 `RSAPrivateKey` nested inside that document.
    let pkcs1 = pkcs8_inner_private_key(private_der)?;
    Ok(encode(&header, claims, &EncodingKey::from_rsa_der(&pkcs1))?)
}

fn jwks_for_rsa(rsa: &RsaKeyPair) -> TestResult<JwkSet> {
    let (modulus, exponent) = rsa_public_components(rsa.public_key().as_ref())?;
    Ok(serde_json::from_value(json!({
        "keys": [{
            "kty": "RSA",
            "kid": KEY_ID,
            "alg": "RS256",
            "use": "sig",
            "n": URL_SAFE_NO_PAD.encode(modulus),
            "e": URL_SAFE_NO_PAD.encode(exponent),
        }]
    }))?)
}

fn pkcs8_inner_private_key(der: &[u8]) -> TestResult<Vec<u8>> {
    let mut index = 0;
    read_tag(der, &mut index, 0x30)?;
    read_len(der, &mut index)?;
    // The PKCS#8 version is INTEGER 0; skip it without `read_integer`, which rejects a
    // value that is empty after stripping leading zeros.
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
