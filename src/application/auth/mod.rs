//! Authentication identities shared by HTTP and realtime boundaries.

mod access_identity;
mod service;

pub use access_identity::{AccessIdentity, AccessTokenVerifier, AuthenticationError};
pub use service::{
    AuthDependencies, AuthError, AuthLifetimePolicy, AuthRateLimitPolicy, AuthService,
    AuthorizeInput, AuthorizeOutput, EndpointRateLimit, ExchangeInput, OAUTH_ATTEMPT_TTL,
    OAuthProviderSlot, SystemAuthClock, TokenPair,
};
