//! Authentication identities shared by HTTP and realtime boundaries.

mod access_identity;

pub use access_identity::{AccessIdentity, AccessTokenVerifier, AuthenticationError};
