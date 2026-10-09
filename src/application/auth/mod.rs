//! Authentication identities shared by HTTP and realtime boundaries.

mod access_identity;
mod service;

pub use access_identity::{
    AccessAccountGate, AccessGateError, AccessGateFuture, AccessIdentity, AccessTokenVerifier,
    AuthenticationError, GatedAccessTokenVerifier,
};
pub use service::{
    AppleExchangeInput, AppleIdentityProviderSlot, AuthDependencies, AuthError, AuthExchangeOutput,
    AuthLifetimePolicy, AuthRateLimitPolicy, AuthService, AuthorizeInput, AuthorizeOutput,
    EndpointRateLimit, ExchangeInput, OAUTH_ATTEMPT_TTL, OAuthProviderSlot, SystemAuthClock,
    TokenPair,
};
