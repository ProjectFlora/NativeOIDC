use std::io;

use serde::{Deserialize, Serialize};

use crate::{parameters::are_parameter_names_unique, AuthError, AuthenticationState};

#[derive(Debug, Deserialize)]
pub(super) struct WindowsAuthorizationRequest {
    #[serde(default)]
    pub(super) issuer: Option<String>,
    #[serde(default)]
    pub(super) authorization_endpoint: Option<String>,
    #[serde(default)]
    pub(super) token_endpoint: Option<String>,
    #[serde(default)]
    pub(super) jwks_uri: Option<String>,
    pub(super) client_id: String,
    pub(super) redirect_uri: String,
    pub(super) scopes: Vec<String>,
    #[serde(default)]
    pub(super) additional_parameters: Vec<WindowsParameter>,
    #[serde(default)]
    pub(super) additional_token_parameters: Vec<WindowsParameter>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WindowsRevocationRequest {
    #[serde(default)]
    pub(super) issuer: Option<String>,
    #[serde(default)]
    pub(super) authorization_endpoint: Option<String>,
    #[serde(default)]
    pub(super) token_endpoint: Option<String>,
    #[serde(default)]
    pub(super) revocation_endpoint: Option<String>,
    pub(super) client_id: String,
    pub(super) token: String,
    #[serde(default)]
    pub(super) token_type_hint: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct WindowsRefreshRequest {
    #[serde(default)]
    pub(super) issuer: Option<String>,
    #[serde(default)]
    pub(super) authorization_endpoint: Option<String>,
    #[serde(default)]
    pub(super) token_endpoint: Option<String>,
    #[serde(default)]
    pub(super) jwks_uri: Option<String>,
    pub(super) client_id: String,
    pub(super) state: AuthenticationState,
    #[serde(default)]
    pub(super) additional_parameters: Vec<WindowsParameter>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WindowsEndSessionRequest {
    #[serde(default)]
    pub(super) issuer: Option<String>,
    #[serde(default)]
    pub(super) authorization_endpoint: Option<String>,
    #[serde(default)]
    pub(super) token_endpoint: Option<String>,
    #[serde(default)]
    pub(super) end_session_endpoint: Option<String>,
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) id_token_hint: Option<String>,
    #[serde(default)]
    pub(super) post_logout_redirect_uri: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct WindowsEndSessionResponse {
    pub(super) state: String,
    pub(super) additional_parameters: Vec<WindowsParameter>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct WindowsParameter {
    pub(super) name: String,
    pub(super) value: String,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum WindowsOperationError {
    #[error(transparent)]
    Core(#[from] AuthError),
    #[error("issuer URL is invalid")]
    InvalidIssuer,
    #[error("an endpoint URL is missing or invalid")]
    InvalidEndpoint,
    #[error("redirect URI must be an unported http://127.0.0.1 loopback URI")]
    InvalidRedirectUri,
    #[error("parameter names must be unique")]
    DuplicateParameter,
    #[cfg(windows)]
    #[error("the system browser could not be opened")]
    BrowserCouldNotBeOpened,
    #[cfg(windows)]
    #[error("could not initialize the async runtime: {0}")]
    RuntimeInitialization(String),
    #[error("loopback server failed: {0}")]
    Loopback(#[from] io::Error),
}

/// The endpoint attachment performed while building a service configuration.
#[derive(Debug, Copy, Clone)]
pub(super) enum ConfigurationKind {
    /// Authorization and token refresh: attach OIDC validation endpoints.
    Oidc,
    /// Token revocation: attach the revocation endpoint.
    Revocation,
    /// End-session: attach the end-session endpoint.
    EndSession,
}

/// How a service configuration was resolved from the request's endpoints.
#[derive(Debug, Copy, Clone)]
pub(super) enum ConfigurationSource {
    /// Both the authorization and token endpoints were supplied explicitly.
    Explicit,
    /// The endpoints were fetched from the issuer's discovery document.
    Discovered,
}

/// Endpoint strings selected by a request, shared by all blocking operations.
#[derive(Debug, Clone, Copy)]
pub(super) struct EndpointSelection<'a> {
    pub(super) issuer: Option<&'a str>,
    pub(super) authorization_endpoint: Option<&'a str>,
    pub(super) token_endpoint: Option<&'a str>,
    /// JWKS URI for ID Token validation; only used by `Oidc` operations.
    pub(super) jwks_uri: Option<&'a str>,
    /// Revocation or end-session endpoint; only used by those operations.
    pub(super) named_endpoint: Option<&'a str>,
}

pub(super) fn ensure_unique_parameter_names(
    parameters: &[WindowsParameter],
) -> std::result::Result<(), WindowsOperationError> {
    if !are_parameter_names_unique(parameters.iter().map(|parameter| parameter.name.as_str())) {
        return Err(WindowsOperationError::DuplicateParameter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenResponse;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use std::collections::BTreeMap;

    #[test]
    fn request_json_accepts_discovery_or_explicit_endpoints() {
        let discovery: WindowsAuthorizationRequest = serde_json::from_str(
            r#"{
                "issuer":"https://issuer.example",
                "client_id":"client",
                "redirect_uri":"http://127.0.0.1/callback",
                "scopes":["profile"]
            }"#,
        )
        .unwrap();
        assert_eq!(discovery.issuer.as_deref(), Some("https://issuer.example"));

        let explicit: WindowsAuthorizationRequest = serde_json::from_str(
            r#"{
                "authorization_endpoint":"https://issuer.example/authorize",
                "token_endpoint":"https://issuer.example/token",
                "client_id":"client",
                "redirect_uri":"http://127.0.0.1/callback",
                "scopes":["profile"]
            }"#,
        )
        .unwrap();
        assert!(explicit.issuer.is_none());
    }

    #[test]
    fn refresh_request_json_round_trips_serialized_state() {
        let state = AuthenticationState::new(TokenResponse {
            access_token: "access".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: Some(3600),
            refresh_token: Some("refresh-token".to_owned()),
            scope: Some("profile".to_owned()),
            id_token: None,
            additional_parameters: BTreeMap::new(),
        })
        .unwrap();
        let serialized_state = serde_json::to_string(&state).unwrap();
        let state_value: serde_json::Value = serde_json::from_str(&serialized_state).unwrap();

        let request: WindowsRefreshRequest = serde_json::from_value(serde_json::json!({
            "authorization_endpoint": "https://issuer.example/authorize",
            "token_endpoint": "https://issuer.example/token",
            "client_id": "client",
            "state": state_value,
        }))
        .unwrap();
        assert_eq!(request.state.token_response().access_token, "access");
        assert_eq!(
            request.state.token_response().refresh_token.as_deref(),
            Some("refresh-token")
        );
        assert!(request.state.expires_at().is_some());

        // State JSON saved before the ID Token claims were recorded gets them
        // from its stored ID Token, and the refreshed state JSON keeps them.
        let payload = serde_json::json!({
            "iss": "https://issuer.example/",
            "sub": "user-123",
            "aud": "client",
            "exp": 2000,
            "iat": 1000,
            "auth_time": 1000,
        });
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let mut state_value = state_value;
        state_value["token_response"]["id_token"] =
            serde_json::json!(format!("e30.{payload}.c2ln"));
        let request: WindowsRefreshRequest = serde_json::from_value(serde_json::json!({
            "token_endpoint": "https://issuer.example/token",
            "client_id": "client",
            "state": state_value,
        }))
        .unwrap();
        let claims = serde_json::json!({
            "iss": "https://issuer.example/",
            "sub": "user-123",
            "aud": ["client"],
            "exp": 2000,
            "iat": 1000,
            "auth_time": 1000,
        });
        let serialized = serde_json::to_value(&request.state).unwrap();
        assert_eq!(serialized["id_token_claims"], claims);
        let restored: AuthenticationState = serde_json::from_value(serialized).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap()["id_token_claims"],
            claims
        );

        let missing_token_response: std::result::Result<WindowsRefreshRequest, _> =
            serde_json::from_value(serde_json::json!({
                "client_id": "client",
                "state": {}
            }));
        assert!(missing_token_response.is_err());
    }
}
