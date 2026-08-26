use jamye_server::{
    adapters::oauth::GoogleIdTokenVerifier, ports::oauth_provider::OAuthProviderError,
};
use jsonwebtoken::jwk::JwkSet;

use crate::TestResult;

// Public-key-only vectors derived from jsonwebtoken's published RSA fixture. Token-shaped
// strings stay split into header, claims, and signature so no reusable bearer value is stored.
const GOOGLE_JWKS: &str = r#"{
  "keys": [
    {
      "alg": "RS256",
      "e": "AQAB",
      "kid": "fixture-key",
      "kty": "RSA",
      "n": "yRE6rHuNR0QbHO3H3Kt2pOKGVhQqGZXInOduQNxXzuKlvQTLUTv4l4sggh5_CYYi_cvI-SXVT9kPWSKXxJXBXd_4LkvcPuUakBoAkfh-eiFVMh2VrUyWyj3MFl0HTVF9KwRXLAcwkREiS3npThHRyIxuy0ZMeZfxVL5arMhw1SRELB8HoGfG_AtH89BIE9jDBHZ9dLelK9a184zAf8LwoPLxvJb3Il5nncqPcSfKDDodMFBIMc4lQzDKL5gvmiXLXB1AGLm8KBjfE8s3L5xqi-yUod-j8MtvIj812dkS4QMiRVN_by2h3ZY8LYVGrqZXZTcgn2ujn8uKjXLZVD5TdQ"
    }
  ]
}"#;

const HEADER_PART: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImZpeHR1cmUta2V5IiwidHlwIjoiSldUIn0";
const VALID_PARTS: (&str, &str) = (
    "eyJzdWIiOiJnb29nbGUtc3ViamVjdCIsIm5vbmNlIjoiZXhwZWN0ZWQtbm9uY2UiLCJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJnb29nbGUtY2xpZW50IiwiaWF0IjoxNzAwMDAwMDAwLCJleHAiOjQxMDI0NDQ4MDB9",
    "lHV3yyWIwQE6cGxRISAc1w0UfeTztaUu3vKevzy6_zq9GlocY6wx6T16R8P6CwyQTR2QAm_l-qmBl5BQj5TN1cr5ME1vr76w2QG06zv2E5PAkh06Wj7JPrk3URRYu9rmqicDTNUZy9_s-4TebpsOcw2f4y2m6PjrMpI06GONAbGXYqhaOopgJqIw3x_jSuYJsVOPdHArnQDf8sUY6RQWxOZQSpmmsTZbQW70owAQEaFtKK0phEYqXEi0EjB2F3N5OKSyTxS8Lj815vmiidpyQEme39eYpGBy5uPhZbqAf1jTAMEhs1xricCppi1mR_yU4JPK7hwQ1LDlk_xKGW_ZZg",
);
const WRONG_ISSUER_PARTS: (&str, &str) = (
    "eyJzdWIiOiJnb29nbGUtc3ViamVjdCIsIm5vbmNlIjoiZXhwZWN0ZWQtbm9uY2UiLCJpc3MiOiJodHRwczovL2V2aWwuZXhhbXBsZSIsImF1ZCI6Imdvb2dsZS1jbGllbnQiLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6NDEwMjQ0NDgwMH0",
    "OvEqFY-uBznexVET91SQs2qRMgMifeXCvkSeUvNDxYmewszfPdSNvUbvUsoZV0BdxtsThE_vRcGHksEJHwUayMt3R6-0QhSNgaJjC4xAAJCev0DfsxWGYFbXARarup-eXfvjwStItsPH1X8wNPvruwGA6VEGbCrD86goI4F3fs2Kqm6ylj2MPh1HqDB_fACWbVoSSViMoOIY9fyCD8XCIUyaBuPGAdoZxY64jCOHqHCPuEFwwvHKnrcYYsz1yN0ynod9qjyl9d5tkr7WSUpozIwTp1C9nFkHQMq_rkHIteDumanxsJ1U9Gd6rjL0pM9DjAYCHhNcMrvL_3n62rccMQ",
);
const WRONG_AUDIENCE_PARTS: (&str, &str) = (
    "eyJzdWIiOiJnb29nbGUtc3ViamVjdCIsIm5vbmNlIjoiZXhwZWN0ZWQtbm9uY2UiLCJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJvdGhlci1jbGllbnQiLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6NDEwMjQ0NDgwMH0",
    "su0oJxcCAgYqmMDcUSyinkiGL2fO2ZbnJWzoWF05cjkZ2klL9NHb0vTWVNiXj4dL0M3sqWJ2g351Cop6If3FjuCdugl-ADIRw-JUX0yTjndMCJrAM2wRTCSMTxbCZwQ6LL0q4RxqVzWbBC7l9i5WGtEg_mlnxjRg1jp4ph2_pP1-Vg61rFqXGbmKQY3uBXMybJsjImHXhb2z1z5Ip4qlXGNfgcOarcdlLfgq_zbwbErD6DvVSraptbSidn4EXPuga4mbvUVHweejFvXSSYrk6cR7ZWD4Fa1GkqntSJk0oxyuJ1D71E9bYhhPS3lp1pH7A2S_s2Yq_Ab7bvxNCjzZpg",
);
const WRONG_NONCE_PARTS: (&str, &str) = (
    "eyJzdWIiOiJnb29nbGUtc3ViamVjdCIsIm5vbmNlIjoid3Jvbmctbm9uY2UiLCJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJnb29nbGUtY2xpZW50IiwiaWF0IjoxNzAwMDAwMDAwLCJleHAiOjQxMDI0NDQ4MDB9",
    "aq-ArvT9OXoxnK8D8y-lFw1W8dGfMQCF2kAdvoule4B69Zq-Uhmg2jViS4w5KdnHc4Zx2102TnTmOJP-xRBCOY9vf-VP6spw4ghJGJxSVq3YUKCP0a9JYA4Qp3RJhfd44BwfQ0gyrHWpNtFeXWn_ml3eJboYFEVicQbuYWQqZ9SVn0c8MT8azs49v5Z7V8XNjF3f0r1eo5crtPN-0iR68NXwegdjFBlLjRvw33YjdIPrcdoEmdEI7r2iC46aB-fDfDxjxeffFFB2G6iId9dstPovCYh99fQq67mznAxsjtdg58KfDb29Xs4GTs2PnupHhKt2BtzAygat7wAsXpmR6w",
);
const EXPIRED_PARTS: (&str, &str) = (
    "eyJzdWIiOiJnb29nbGUtc3ViamVjdCIsIm5vbmNlIjoiZXhwZWN0ZWQtbm9uY2UiLCJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJnb29nbGUtY2xpZW50IiwiaWF0IjoxLCJleHAiOjJ9",
    "Z6vwWRlgzDlx5lh0VYHVTRdeG3F8NWT9eR62N1X-ER3qRi2oiQxnxLOyBMfkcZOLX5kQ1C-N5NOjUdzO_3hDVSVHnb_euKYIdbNc2LN3jLD22wCCStcwO9J9Auv4lulGbyEpnWQju270YCGRQ6f8oY5-mJzaIUhUyTM6h5ku8HQRLK13GE0noakSy8s9QsvrwfZhIVyS3vlHpGkvAXTuzuBrywsBnHNt-HAQX4N8MmPvvmLW2na_uCe0S51d2eJFnM_gHY6ZuMdXgr6crmy4FrAgc38qVZrxSh8zbqUmCsikyF4VFwrI1QPMiQv83UJzIqPzLdl2_hrvJquYx8yfJA",
);

#[test]
fn google_id_token_rejects_wrong_issuer_audience_nonce_signature_and_expiry() -> TestResult {
    let jwks = serde_json::from_str::<JwkSet>(GOOGLE_JWKS)?;
    let verifier = GoogleIdTokenVerifier::new("google-client")?;
    let valid = token(VALID_PARTS);
    assert_eq!(
        verifier.verify_subject(&valid, "expected-nonce", &jwks)?,
        "google-subject"
    );
    for parts in [
        WRONG_ISSUER_PARTS,
        WRONG_AUDIENCE_PARTS,
        WRONG_NONCE_PARTS,
        EXPIRED_PARTS,
    ] {
        let invalid = token(parts);
        assert_eq!(
            verifier.verify_subject(&invalid, "expected-nonce", &jwks),
            Err(OAuthProviderError::InvalidIdentity)
        );
    }

    let mut tampered = valid;
    let final_character = tampered
        .pop()
        .ok_or_else(|| std::io::Error::other("Google token fixture is empty"))?;
    tampered.push(if final_character == 'A' { 'B' } else { 'A' });
    assert_eq!(
        verifier.verify_subject(&tampered, "expected-nonce", &jwks),
        Err(OAuthProviderError::InvalidIdentity)
    );
    Ok(())
}

fn token(parts: (&str, &str)) -> String {
    format!("{HEADER_PART}.{}.{}", parts.0, parts.1)
}
