use url::Url;

use super::validation::{
    parse_issuer, validate_end_session_endpoint, validate_endpoint, validate_jwks_endpoint,
    validate_registration_endpoint, validate_revocation_endpoint,
};
use crate::{id_token::JwksCache, Result};

#[derive(Clone)]
/// Authorization, token, OIDC, registration, and end-session endpoint configuration.
pub struct ServiceConfiguration {
    pub(super) authorization_endpoint: Url,
    pub(super) token_endpoint: Url,
    pub(super) revocation_endpoint: Option<Url>,
    pub(super) end_session_endpoint: Option<Url>,
    pub(super) registration_endpoint: Option<Url>,
    pub(super) issuer: Option<String>,
    pub(super) jwks_uri: Option<Url>,
    pub(super) jwks_cache: JwksCache,
}

impl ServiceConfiguration {
    /// Creates a configuration from explicit authorization and token endpoints.
    pub fn new(authorization_endpoint: Url, token_endpoint: Url) -> Result<Self> {
        validate_endpoint(&authorization_endpoint)?;
        validate_endpoint(&token_endpoint)?;

        Ok(Self {
            authorization_endpoint,
            token_endpoint,
            revocation_endpoint: None,
            end_session_endpoint: None,
            registration_endpoint: None,
            issuer: None,
            jwks_uri: None,
            jwks_cache: JwksCache::default(),
        })
    }

    /// Adds an OAuth 2.0 token revocation endpoint. It must use HTTPS, or HTTP
    /// on a loopback host (`127.0.0.0/8`, `::1`, or `localhost`).
    pub fn with_revocation_endpoint(mut self, endpoint: Url) -> Result<Self> {
        validate_revocation_endpoint(&endpoint)?;
        self.revocation_endpoint = Some(endpoint);
        Ok(self)
    }

    /// Adds an HTTPS end-session endpoint.
    pub fn with_end_session_endpoint(mut self, endpoint: Url) -> Result<Self> {
        validate_end_session_endpoint(&endpoint)?;
        self.end_session_endpoint = Some(endpoint);
        Ok(self)
    }

    /// Adds an HTTPS dynamic client registration endpoint.
    pub fn with_registration_endpoint(mut self, endpoint: Url) -> Result<Self> {
        validate_registration_endpoint(&endpoint)?;
        self.registration_endpoint = Some(endpoint);
        Ok(self)
    }

    /// Configures the issuer and JWKS endpoint used to validate ID Tokens.
    ///
    /// The issuer identifier is kept exactly as given and compared with the ID
    /// Token `iss` claim by simple string comparison, as OpenID Connect Core
    /// requires. No trailing slash is added or removed, so
    /// `https://issuer.example` and `https://issuer.example/` are different
    /// issuers. Passing a [`Url`] uses its serialization, which always has a
    /// path; pass a string for an issuer without a path. The issuer must be an
    /// HTTPS URL without user info, query, fragment, whitespace, or control
    /// characters.
    pub fn with_oidc_validation_endpoints(
        mut self,
        issuer: impl AsRef<str>,
        jwks_uri: Url,
    ) -> Result<Self> {
        let issuer = issuer.as_ref();
        parse_issuer(issuer)?;
        validate_jwks_endpoint(&jwks_uri)?;
        self.issuer = Some(issuer.to_owned());
        self.jwks_uri = Some(jwks_uri);
        self.jwks_cache = JwksCache::default();
        Ok(self)
    }

    /// Returns the configured authorization endpoint.
    pub fn authorization_endpoint(&self) -> &Url {
        &self.authorization_endpoint
    }

    /// Returns the configured token endpoint.
    pub fn token_endpoint(&self) -> &Url {
        &self.token_endpoint
    }

    /// Returns the configured token revocation endpoint, if any.
    pub fn revocation_endpoint(&self) -> Option<&Url> {
        self.revocation_endpoint.as_ref()
    }

    /// Returns the configured end-session endpoint, if any.
    pub fn end_session_endpoint(&self) -> Option<&Url> {
        self.end_session_endpoint.as_ref()
    }

    /// Returns the configured client registration endpoint, if any.
    pub fn registration_endpoint(&self) -> Option<&Url> {
        self.registration_endpoint.as_ref()
    }

    /// Returns the configured OIDC issuer identifier, if any, exactly as it is
    /// compared with the ID Token `iss` claim.
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }

    /// Returns the configured JWKS endpoint, if any.
    pub fn jwks_uri(&self) -> Option<&Url> {
        self.jwks_uri.as_ref()
    }
}
