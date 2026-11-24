use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    http::{send_token_request, serialize_token_form},
    json::{take_optional_string, take_optional_u64, take_required_string, UniqueJsonObject},
    validation::validate_additional_token_parameters,
    ServiceConfiguration,
};
use crate::{
    id_token::validate_id_token_if_present, AuthError, CancellationHandle, HttpTransport, Result,
};

#[derive(Serialize, Deserialize)]
/// Token values returned by an OAuth token endpoint.
pub struct TokenResponse {
    /// The bearer access token.
    pub access_token: String,
    /// The token type, typically `Bearer`.
    pub token_type: String,
    /// Access-token lifetime in seconds, if supplied.
    pub expires_in: Option<u64>,
    /// Refresh token, if supplied.
    pub refresh_token: Option<String>,
    /// Granted scopes, if supplied.
    pub scope: Option<String>,
    /// Raw ID Token value. Tokens returned by code exchange are verified before return.
    pub id_token: Option<String>,
    #[serde(default)]
    /// Unrecognized token response values preserved as JSON.
    pub additional_parameters: BTreeMap<String, Value>,
}

/// Serializable token state. The host application remains responsible for secure storage.
#[derive(Serialize, Deserialize)]
pub struct AuthenticationState {
    token_response: TokenResponse,
    expires_at: Option<u64>,
    #[serde(default)]
    oidc_nonce: Option<String>,
}

impl AuthenticationState {
    /// Creates state and records expiry relative to the current system time.
    ///
    /// Use [`AuthorizationSession::authentication_state`] when the state
    /// follows an OIDC authorization and must retain that request's nonce.
    ///
    /// [`AuthorizationSession::authentication_state`]: super::AuthorizationSession::authentication_state
    pub fn new(token_response: TokenResponse) -> Result<Self> {
        Self::new_with_oidc_nonce(token_response, None)
    }

    /// Creates state while retaining the nonce used for the OIDC authorization.
    ///
    /// This state contains sensitive tokens. The host application must encrypt
    /// it before persisting it.
    pub fn new_with_oidc_nonce(
        token_response: TokenResponse,
        oidc_nonce: Option<String>,
    ) -> Result<Self> {
        if oidc_nonce
            .as_deref()
            .is_some_and(|nonce| nonce.trim().is_empty())
        {
            return Err(AuthError::InvalidConfiguration(
                "OIDC nonce must not be empty",
            ));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AuthError::Clock)?;
        Self::new_at_with_oidc_nonce(token_response, now.as_secs(), oidc_nonce)
    }

    #[cfg(test)]
    pub(super) fn new_at(token_response: TokenResponse, now: u64) -> Result<Self> {
        Self::new_at_with_oidc_nonce(token_response, now, None)
    }

    fn new_at_with_oidc_nonce(
        token_response: TokenResponse,
        now: u64,
        oidc_nonce: Option<String>,
    ) -> Result<Self> {
        let expires_at = token_response
            .expires_in
            .map(|expires_in| {
                now.checked_add(expires_in)
                    .ok_or(AuthError::InvalidTokenResponse(
                        "expires_in exceeds the supported range",
                    ))
            })
            .transpose()?;
        Ok(Self {
            token_response,
            expires_at,
            oidc_nonce,
        })
    }

    /// Returns the current token response.
    pub fn token_response(&self) -> &TokenResponse {
        &self.token_response
    }

    /// Returns the access-token expiration as seconds since the Unix epoch.
    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    /// Returns `None` when the token response did not include `expires_in`.
    pub fn is_expired_at(&self, unix_time_seconds: u64) -> Option<bool> {
        self.expires_at
            .map(|expires_at| unix_time_seconds >= expires_at)
    }

    /// Refreshes explicitly and only updates this state after a successful response.
    ///
    /// No automatic refresh is performed. If the operation fails, the existing
    /// state remains unchanged.
    pub async fn refresh(
        &mut self,
        configuration: &ServiceConfiguration,
        client_id: &str,
        transport: &dyn HttpTransport,
        cancellation: CancellationHandle,
    ) -> Result<()> {
        self.refresh_with_additional_parameters(
            configuration,
            client_id,
            &BTreeMap::new(),
            transport,
            cancellation,
        )
        .await
    }

    /// Refreshes using additional token endpoint parameters.
    ///
    /// No automatic refresh is performed. If the operation fails, the existing
    /// state remains unchanged.
    pub async fn refresh_with_additional_parameters(
        &mut self,
        configuration: &ServiceConfiguration,
        client_id: &str,
        additional_parameters: &BTreeMap<String, String>,
        transport: &dyn HttpTransport,
        cancellation: CancellationHandle,
    ) -> Result<()> {
        if client_id.trim().is_empty() {
            return Err(AuthError::InvalidConfiguration(
                "client ID must not be empty",
            ));
        }
        validate_additional_token_parameters(additional_parameters)?;
        let refresh_token = self
            .token_response
            .refresh_token
            .as_deref()
            .filter(|refresh_token| !refresh_token.trim().is_empty())
            .ok_or(AuthError::InvalidConfiguration(
                "authentication state has no refresh token",
            ))?;

        let body = serialize_token_form(
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", client_id),
            ],
            additional_parameters,
        );
        let mut refreshed = send_token_request(
            &configuration.token_endpoint,
            body,
            transport,
            cancellation.clone(),
        )
        .await?;
        validate_id_token_if_present(
            refreshed.id_token.as_deref(),
            configuration.issuer.as_ref(),
            configuration.jwks_uri.as_ref(),
            client_id,
            self.oidc_nonce.as_deref(),
            false,
            &configuration.jwks_cache,
            transport,
            cancellation,
        )
        .await?;

        if refreshed.refresh_token.is_none() {
            refreshed.refresh_token = Some(refresh_token.to_owned());
        }
        if refreshed.scope.is_none() {
            refreshed.scope = self.token_response.scope.clone();
        }
        if refreshed.id_token.is_none() {
            refreshed.id_token = self.token_response.id_token.clone();
        }

        let mut updated_state = Self::new(refreshed)?;
        updated_state.oidc_nonce = self.oidc_nonce.clone();
        *self = updated_state;
        Ok(())
    }
}

pub(super) fn parse_token_response(body: &[u8]) -> Result<TokenResponse> {
    let UniqueJsonObject(mut object) = serde_json::from_slice(body).map_err(|_| {
        AuthError::InvalidTokenResponse("response is not a JSON object with unique member names")
    })?;
    if object.contains_key("error") {
        return Err(AuthError::InvalidTokenResponse(
            "successful token response contains an error",
        ));
    }

    let access_token = take_required_string(&mut object, "access_token")?;
    let token_type = take_required_string(&mut object, "token_type")?;
    if access_token.is_empty() || token_type.is_empty() {
        return Err(AuthError::InvalidTokenResponse(
            "access_token and token_type must not be empty",
        ));
    }

    let expires_in = take_optional_u64(&mut object, "expires_in")?;
    let refresh_token = take_optional_string(&mut object, "refresh_token")?;
    if matches!(refresh_token.as_deref(), Some("")) {
        return Err(AuthError::InvalidTokenResponse(
            "refresh_token must not be empty",
        ));
    }
    let scope = take_optional_string(&mut object, "scope")?;
    let id_token = take_optional_string(&mut object, "id_token")?;

    Ok(TokenResponse {
        access_token,
        token_type,
        expires_in,
        refresh_token,
        scope,
        id_token,
        additional_parameters: object.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use http::{Method, StatusCode};
    use url::{form_urlencoded, Url};

    use super::*;
    use crate::authorization::{test_support::*, validation::RESERVED_TOKEN_PARAMETERS};

    #[test]
    fn authentication_state_serializes_expiration() {
        let state = authentication_state();

        assert_eq!(state.expires_at(), Some(1_060));
        assert_eq!(state.is_expired_at(1_059), Some(false));
        assert_eq!(state.is_expired_at(1_060), Some(true));

        let serialized = serde_json::to_vec(&state).unwrap();
        let restored: AuthenticationState = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored.expires_at(), Some(1_060));
        assert_eq!(
            restored.token_response().access_token,
            state.token_response().access_token
        );

        let token_response = TokenResponse {
            access_token: "access".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: None,
            refresh_token: None,
            scope: None,
            id_token: None,
            additional_parameters: BTreeMap::new(),
        };
        let state_without_expiry = AuthenticationState::new_at(token_response, 1_000).unwrap();
        assert_eq!(state_without_expiry.expires_at(), None);
        assert_eq!(state_without_expiry.is_expired_at(2_000), None);
    }

    #[test]
    fn authorization_session_carries_nonce_into_serialized_authentication_state() {
        let session = request(&["openid"]).begin().unwrap();
        let state = session
            .authentication_state(TokenResponse {
                access_token: "access".to_owned(),
                token_type: "Bearer".to_owned(),
                expires_in: None,
                refresh_token: Some("refresh".to_owned()),
                scope: Some("openid".to_owned()),
                id_token: Some("validated-id-token".to_owned()),
                additional_parameters: BTreeMap::new(),
            })
            .unwrap();

        let serialized = serde_json::to_value(&state).unwrap();
        assert_eq!(
            serialized["oidc_nonce"],
            serde_json::json!(session.nonce().unwrap())
        );
        let restored: AuthenticationState = serde_json::from_value(serialized).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap()["oidc_nonce"],
            serde_json::json!(session.nonce().unwrap())
        );
    }

    #[tokio::test]
    async fn refresh_updates_tokens_and_retains_omitted_refresh_fields() {
        let mut state = authentication_state();
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let transport = test_transport(
            StatusCode::OK,
            br#"{
                "access_token":"new-access",
                "token_type":"Bearer",
                "expires_in":3600
            }"#,
        );
        let additional_parameters =
            BTreeMap::from([("audience".to_owned(), "https://api.example".to_owned())]);

        state
            .refresh_with_additional_parameters(
                &service_configuration(),
                "native-client",
                &additional_parameters,
                &transport,
                CancellationHandle::new(),
            )
            .await
            .unwrap();

        let request = transport.request.lock().unwrap().take().unwrap();
        let form: BTreeMap<_, _> = form_urlencoded::parse(&request.body).into_owned().collect();
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.url.as_str(), "https://issuer.example/token");
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            form.get("refresh_token").map(String::as_str),
            Some("old-refresh")
        );
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some("native-client")
        );
        assert_eq!(
            form.get("audience").map(String::as_str),
            Some("https://api.example")
        );
        assert_eq!(state.token_response().access_token, "new-access");
        assert_eq!(
            state.token_response().refresh_token.as_deref(),
            Some("old-refresh")
        );
        assert_eq!(
            state.token_response().scope.as_deref(),
            Some("openid profile")
        );
        assert_eq!(state.token_response().id_token.as_deref(), Some("old-id"));
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expires_at = state.expires_at().unwrap();
        assert!(expires_at >= before + 3_600 && expires_at <= after + 3_600);
    }

    #[tokio::test]
    async fn refresh_rotates_refresh_token_and_preserves_state_on_failure() {
        let mut state = authentication_state();
        let transport = test_transport(
            StatusCode::OK,
            br#"{
                "access_token":"new-access",
                "token_type":"Bearer",
                "refresh_token":"rotated-refresh"
            }"#,
        );
        state
            .refresh(
                &service_configuration(),
                "native-client",
                &transport,
                CancellationHandle::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            state.token_response().refresh_token.as_deref(),
            Some("rotated-refresh")
        );

        let before = serde_json::to_value(&state).unwrap();
        let error_transport =
            test_transport(StatusCode::BAD_REQUEST, br#"{"error":"invalid_grant"}"#);
        assert!(matches!(
            state
                .refresh(
                    &service_configuration(),
                    "native-client",
                    &error_transport,
                    CancellationHandle::new(),
                )
                .await,
            Err(AuthError::AuthorizationServer { code, .. }) if code == "invalid_grant"
        ));
        assert_eq!(serde_json::to_value(&state).unwrap(), before);

        let invalid_response = test_transport(
            StatusCode::OK,
            br#"{
                "access_token":"new-access",
                "token_type":"Bearer",
                "refresh_token":""
            }"#,
        );
        assert!(matches!(
            state
                .refresh(
                    &service_configuration(),
                    "native-client",
                    &invalid_response,
                    CancellationHandle::new(),
                )
                .await,
            Err(AuthError::InvalidTokenResponse(
                "refresh_token must not be empty"
            ))
        ));
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }

    #[tokio::test]
    async fn refresh_rejects_an_invalid_returned_id_token_without_updating_state() {
        let mut state = AuthenticationState::new_with_oidc_nonce(
            TokenResponse {
                access_token: "old-access".to_owned(),
                token_type: "Bearer".to_owned(),
                expires_in: None,
                refresh_token: Some("old-refresh".to_owned()),
                scope: Some("openid".to_owned()),
                id_token: Some("old-id".to_owned()),
                additional_parameters: BTreeMap::new(),
            },
            Some("original-nonce".to_owned()),
        )
        .unwrap();
        let before = serde_json::to_value(&state).unwrap();
        let configuration = service_configuration()
            .with_oidc_validation_endpoints(
                Url::parse("https://issuer.example/").unwrap(),
                Url::parse("https://issuer.example/keys").unwrap(),
            )
            .unwrap();
        let transport = test_transport(
            StatusCode::OK,
            br#"{
                "access_token":"new-access",
                "token_type":"Bearer",
                "id_token":"not.a.jwt"
            }"#,
        );

        assert!(matches!(
            state
                .refresh(
                    &configuration,
                    "native-client",
                    &transport,
                    CancellationHandle::new(),
                )
                .await,
            Err(AuthError::InvalidIdToken(
                "JWT header is malformed or unsupported"
            ))
        ));
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }

    #[tokio::test]
    async fn refresh_without_a_refresh_token_does_not_send_a_request() {
        let mut state = AuthenticationState::new_at(
            TokenResponse {
                access_token: "access".to_owned(),
                token_type: "Bearer".to_owned(),
                expires_in: None,
                refresh_token: None,
                scope: None,
                id_token: None,
                additional_parameters: BTreeMap::new(),
            },
            1_000,
        )
        .unwrap();
        let transport = test_transport(StatusCode::OK, br#"{}"#);

        assert!(matches!(
            state
                .refresh(
                    &service_configuration(),
                    "native-client",
                    &transport,
                    CancellationHandle::new(),
                )
                .await,
            Err(AuthError::InvalidConfiguration(
                "authentication state has no refresh token"
            ))
        ));
        assert!(transport.request.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn refresh_rejects_reserved_additional_parameters_before_sending() {
        let mut state = authentication_state();
        let additional_parameters =
            BTreeMap::from([("client_secret".to_owned(), "not-allowed".to_owned())]);
        let transport = test_transport(StatusCode::OK, br#"{}"#);

        assert!(matches!(
            state
                .refresh_with_additional_parameters(
                    &service_configuration(),
                    "native-client",
                    &additional_parameters,
                    &transport,
                    CancellationHandle::new(),
                )
                .await,
            Err(AuthError::InvalidConfiguration(
                "additional token parameter has an empty or reserved name"
            ))
        ));
        assert!(transport.request.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn refresh_rejects_every_reserved_extra_name_without_changing_state() {
        for name in RESERVED_TOKEN_PARAMETERS
            .iter()
            .copied()
            .chain(std::iter::once(""))
        {
            let transport = test_transport(StatusCode::OK, b"unused");
            let mut state = authentication_state();
            let before = serde_json::to_value(&state).unwrap();
            let parameters = BTreeMap::from([(name.to_owned(), "override".to_owned())]);
            assert!(matches!(
                state
                    .refresh_with_additional_parameters(
                        &service_configuration(),
                        "client",
                        &parameters,
                        &transport,
                        CancellationHandle::new()
                    )
                    .await,
                Err(AuthError::InvalidConfiguration(_))
            ));
            assert_eq!(serde_json::to_value(&state).unwrap(), before);
            assert!(transport.request.lock().unwrap().is_none());
        }
    }

    #[test]
    fn token_response_rejects_duplicate_members_and_success_error_conflicts() {
        for body in [
            r#"{"access_token":"first","access_token":"second","token_type":"Bearer"}"#,
            r#"{"access_token":"first","access_\u0074oken":"second","token_type":"Bearer"}"#,
            r#"{"access_token":"value","token_type":"Bearer","expires_in":1,"expires_in":2}"#,
            r#"{"access_token":"value","token_type":"Bearer","extension":1,"extension":2}"#,
            r#"{"access_token":"value","token_type":"Bearer","error":"invalid_grant"}"#,
            r#"{"access_token":"value","token_type":"Bearer","error":null}"#,
            r#"[]"#,
            r#"{"access_token":"value","token_type":"Bearer"} {}"#,
        ] {
            assert!(
                matches!(
                    parse_token_response(body.as_bytes()),
                    Err(AuthError::InvalidTokenResponse(_))
                ),
                "accepted {body}"
            );
        }
    }

    #[test]
    fn token_response_preserves_nested_extension_values() {
        let response = parse_token_response(br#"{"access_token":"value","token_type":"Bearer","extension":{"items":[1,null,{"enabled":true}]}}"#).unwrap();
        assert_eq!(
            response.additional_parameters["extension"],
            serde_json::json!({"items": [1, null, {"enabled": true}]})
        );
    }
}
