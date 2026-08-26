use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use crate::ports::auth::{
    CredentialDigest, CredentialError, CredentialSource, GeneratedCredential, RawCredential,
};

const CREDENTIAL_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Default)]
pub struct OsCredentialSource;

impl CredentialSource for OsCredentialSource {
    fn generate(&self) -> Result<GeneratedCredential, CredentialError> {
        let mut bytes = [0_u8; CREDENTIAL_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| CredentialError)?;
        let raw = URL_SAFE_NO_PAD.encode(bytes);
        Ok(GeneratedCredential {
            digest: digest(&raw),
            raw: RawCredential::new(raw),
        })
    }

    fn digest(&self, raw: &str) -> Result<CredentialDigest, CredentialError> {
        Ok(digest(raw))
    }

    fn pkce_s256(&self, verifier: &str) -> Result<String, CredentialError> {
        Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())))
    }
}

fn digest(raw: &str) -> CredentialDigest {
    CredentialDigest::new(Sha256::digest(raw.as_bytes()).into())
}
