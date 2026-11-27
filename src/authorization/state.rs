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
    id_token::{validate_id_token_if_present, ValidatedIdTokenClaims},
    AuthError, CancellationHandle, HttpTransport, Result,
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
///
/// The state keeps the identity claims (`iss`, `sub`, `aud`, `azp`, and
/// `auth_time`) of its ID Token so that refresh can reject a returned ID Token
/// for a different identity. State saved without these claims takes them from
/// its stored ID Token when it is deserialized.
#[derive(Serialize, Deserialize)]
#[serde(from = "SerializedAuthenticationState")]
pub struct AuthenticationState {
    token_response: TokenResponse,
    expires_at: Option<u64>,
    #[serde(default)]
    oidc_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id_token_claims: Option<ValidatedIdTokenClaims>,
}

/// The serialized form of [`AuthenticationState`], including state saved by
/// versions that did not record the ID Token claims.
#[derive(Deserialize)]
struct SerializedAuthenticationState {
    token_response: TokenResponse,
    expires_at: Option<u64>,
    #[serde(default)]
    oidc_nonce: Option<String>,
    #[serde(default)]
    id_token_claims: Option<ValidatedIdTokenClaims>,
}

impl From<SerializedAuthenticationState> for AuthenticationState {
    fn from(serialized: SerializedAuthenticationState) -> Self {
        // State saved before the claims were recorded takes its baseline from
        // the ID Token it stores, which this library validated when it was
        // returned. Without a readable stored ID Token, the next ID Token
        // validated on refresh becomes the baseline.
        let id_token_claims = serialized
            .id_token_claims
            .or_else(|| stored_id_token_claims(&serialized.token_response));
        Self {
            token_response: serialized.token_response,
            expires_at: serialized.expires_at,
            oidc_nonce: serialized.oidc_nonce,
            id_token_claims,
        }
    }
}

fn stored_id_token_claims(token_response: &TokenResponse) -> Option<ValidatedIdTokenClaims> {
    token_response
        .id_token
        .as_deref()
        .and_then(ValidatedIdTokenClaims::read_stored)
}

/// Checks a refreshed ID Token against the original one (OIDC Core 12.2) and
/// returns the claims to keep.
///
/// `iss`, `sub`, and `aud` must be the same. A present `auth_time` must equal
/// the original one; a refreshed ID Token without `auth_time` keeps the
/// original value. `azp` is checked against the client ID during validation
/// and need not be present in both tokens.
fn merge_refreshed_id_token_claims(
    original: &ValidatedIdTokenClaims,
    refreshed: ValidatedIdTokenClaims,
) -> Result<ValidatedIdTokenClaims> {
    if refreshed.iss != original.iss {
        return Err(AuthError::InvalidIdToken(
            "refreshed ID Token issuer does not match the original",
        ));
    }
    if refreshed.sub != original.sub {
        return Err(AuthError::InvalidIdToken(
            "refreshed ID Token subject does not match the original",
        ));
    }
    if refreshed.aud != original.aud {
        return Err(AuthError::InvalidIdToken(
            "refreshed ID Token audience does not match the original",
        ));
    }
    if let (Some(original_auth_time), Some(refreshed_auth_time)) =
        (original.auth_time, refreshed.auth_time)
    {
        if original_auth_time != refreshed_auth_time {
            return Err(AuthError::InvalidIdToken(
                "refreshed ID Token auth_time does not match the original",
            ));
        }
    }
    Ok(ValidatedIdTokenClaims {
        iss: refreshed.iss,
        sub: refreshed.sub,
        aud: refreshed.aud,
        azp: refreshed.azp.or_else(|| original.azp.clone()),
        auth_time: original.auth_time.or(refreshed.auth_time),
    })
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
        let id_token_claims = stored_id_token_claims(&token_response);
        Ok(Self {
            token_response,
            expires_at,
            oidc_nonce,
            id_token_claims,
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
    /// A returned ID Token is validated and must have the same `iss`, `sub`,
    /// and `aud` as the original one, and the same `auth_time` when it
    /// contains one.
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
        let refreshed_claims = validate_id_token_if_present(
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
        let id_token_claims = match refreshed_claims {
            Some(claims) => Some(match &self.id_token_claims {
                Some(original) => merge_refreshed_id_token_claims(original, claims)?,
                None => claims,
            }),
            None => self.id_token_claims.clone(),
        };

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
        updated_state.id_token_claims = id_token_claims;
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
    use crate::{
        authorization::{test_support::*, validation::RESERVED_TOKEN_PARAMETERS},
        id_token::tests::{jwks, ScriptedTransport, TestKey},
        HttpResponse,
    };

    fn unix_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn oidc_configuration(issuer: &str) -> ServiceConfiguration {
        service_configuration()
            .with_oidc_validation_endpoints(
                Url::parse(issuer).unwrap(),
                Url::parse("https://issuer.example/keys").unwrap(),
            )
            .unwrap()
    }

    fn id_token_claims(subject: &str, auth_time: Option<u64>) -> Value {
        let now = unix_now();
        let mut claims = serde_json::json!({
            "iss": "https://issuer.example/",
            "sub": subject,
            "aud": "native-client",
            "exp": now + 300,
            "iat": now,
        });
        if let Some(auth_time) = auth_time {
            claims["auth_time"] = serde_json::json!(auth_time);
        }
        claims
    }

    fn oidc_state(id_token: Option<String>) -> AuthenticationState {
        AuthenticationState::new(TokenResponse {
            access_token: "old-access".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: None,
            refresh_token: Some("old-refresh".to_owned()),
            scope: Some("openid".to_owned()),
            id_token,
            additional_parameters: BTreeMap::new(),
        })
        .unwrap()
    }

    fn ok_response(body: Value) -> Result<HttpResponse> {
        Ok(HttpResponse {
            status: StatusCode::OK,
            headers: http::HeaderMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }

    // Returns the token response, then the JWKS when an ID Token is returned.
    fn refresh_transport(id_token: Option<&str>, key: &TestKey) -> ScriptedTransport {
        let mut token_response = serde_json::json!({
            "access_token": "new-access",
            "token_type": "Bearer",
        });
        let mut responses = Vec::new();
        if let Some(id_token) = id_token {
            token_response["id_token"] = serde_json::json!(id_token);
            responses.push(ok_response(token_response));
            responses.push(ok_response(jwks(&[key])));
        } else {
            responses.push(ok_response(token_response));
        }
        ScriptedTransport::new(responses, false)
    }

    async fn refresh_with(
        state: &mut AuthenticationState,
        issuer: &str,
        transport: &ScriptedTransport,
    ) -> Result<()> {
        state
            .refresh(
                &oidc_configuration(issuer),
                "native-client",
                transport,
                CancellationHandle::new(),
            )
            .await
    }

    #[tokio::test]
    async fn refresh_rejects_an_id_token_whose_identity_differs_from_the_original() {
        let key = TestKey::rs256();
        let auth_time = unix_now() - 10;
        let original = key.sign(id_token_claims("user-123", Some(auth_time)));
        let other_issuer = {
            let mut claims = id_token_claims("user-123", Some(auth_time));
            claims["iss"] = serde_json::json!("https://other.example/");
            claims
        };
        let other_subject = id_token_claims("someone-else", Some(auth_time));
        let other_audience = {
            let mut claims = id_token_claims("user-123", Some(auth_time));
            claims["aud"] = serde_json::json!(["native-client", "another-client"]);
            claims["azp"] = serde_json::json!("native-client");
            claims
        };
        let other_auth_time = id_token_claims("user-123", Some(auth_time + 1));
        // The #260 reproduction: another subject, a new auth_time, and an azp.
        let reproduction = {
            let mut claims = id_token_claims("someone-else", Some(auth_time + 10));
            claims["azp"] = serde_json::json!("native-client");
            claims
        };
        for (issuer, refreshed, message) in [
            (
                "https://other.example/",
                other_issuer,
                "refreshed ID Token issuer does not match the original",
            ),
            (
                "https://issuer.example/",
                other_subject,
                "refreshed ID Token subject does not match the original",
            ),
            (
                "https://issuer.example/",
                other_audience,
                "refreshed ID Token audience does not match the original",
            ),
            (
                "https://issuer.example/",
                other_auth_time,
                "refreshed ID Token auth_time does not match the original",
            ),
            (
                "https://issuer.example/",
                reproduction,
                "refreshed ID Token subject does not match the original",
            ),
        ] {
            let mut state = oidc_state(Some(original.clone()));
            let before = serde_json::to_value(&state).unwrap();
            let transport = refresh_transport(Some(&key.sign(refreshed)), &key);
            let result = refresh_with(&mut state, issuer, &transport).await;
            assert!(
                matches!(result, Err(AuthError::InvalidIdToken(actual)) if actual == message),
                "{message}: {result:?}"
            );
            assert_eq!(serde_json::to_value(&state).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn refresh_accepts_an_id_token_for_the_same_identity_and_keeps_the_auth_time() {
        let key = TestKey::rs256();
        let auth_time = unix_now() - 10;
        let original = key.sign(id_token_claims("user-123", Some(auth_time)));

        // Same claims, audience as a one-element array, and no auth_time.
        let mut without_auth_time = id_token_claims("user-123", None);
        without_auth_time["aud"] = serde_json::json!(["native-client"]);
        without_auth_time["azp"] = serde_json::json!("native-client");
        let without_auth_time = key.sign(without_auth_time);
        let mut state = oidc_state(Some(original));
        let transport = refresh_transport(Some(&without_auth_time), &key);
        refresh_with(&mut state, "https://issuer.example/", &transport)
            .await
            .unwrap();
        assert_eq!(
            state.token_response().id_token.as_deref(),
            Some(without_auth_time.as_str())
        );
        assert_eq!(
            serde_json::to_value(&state).unwrap()["id_token_claims"],
            serde_json::json!({
                "iss": "https://issuer.example/",
                "sub": "user-123",
                "aud": ["native-client"],
                "azp": "native-client",
                "auth_time": auth_time,
            })
        );

        // The original auth_time still applies to the next refresh.
        let same_auth_time = key.sign(id_token_claims("user-123", Some(auth_time)));
        let transport = refresh_transport(Some(&same_auth_time), &key);
        refresh_with(&mut state, "https://issuer.example/", &transport)
            .await
            .unwrap();
        let other_auth_time = key.sign(id_token_claims("user-123", Some(auth_time + 1)));
        let transport = refresh_transport(Some(&other_auth_time), &key);
        assert!(matches!(
            refresh_with(&mut state, "https://issuer.example/", &transport).await,
            Err(AuthError::InvalidIdToken(
                "refreshed ID Token auth_time does not match the original"
            ))
        ));
    }

    #[tokio::test]
    async fn refresh_without_an_id_token_keeps_the_original_claims() {
        let key = TestKey::rs256();
        let original = key.sign(id_token_claims("user-123", Some(unix_now() - 10)));
        let mut state = oidc_state(Some(original.clone()));
        let claims = serde_json::to_value(&state).unwrap()["id_token_claims"].clone();
        assert_eq!(claims["sub"], "user-123");

        let transport = refresh_transport(None, &key);
        refresh_with(&mut state, "https://issuer.example/", &transport)
            .await
            .unwrap();
        assert_eq!(state.token_response().access_token, "new-access");
        assert_eq!(
            state.token_response().id_token.as_deref(),
            Some(original.as_str())
        );
        assert_eq!(
            serde_json::to_value(&state).unwrap()["id_token_claims"],
            claims
        );
    }

    #[tokio::test]
    async fn state_saved_without_claims_takes_them_from_its_stored_id_token() {
        let key = TestKey::rs256();
        let original = key.sign(id_token_claims("user-123", Some(unix_now() - 10)));
        let state = oidc_state(Some(original));
        let mut serialized = serde_json::to_value(&state).unwrap();
        let claims = serialized
            .as_object_mut()
            .unwrap()
            .remove("id_token_claims")
            .unwrap();

        let mut restored: AuthenticationState = serde_json::from_value(serialized).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap()["id_token_claims"],
            claims
        );
        let other_subject = key.sign(id_token_claims("someone-else", None));
        let transport = refresh_transport(Some(&other_subject), &key);
        assert!(matches!(
            refresh_with(&mut restored, "https://issuer.example/", &transport).await,
            Err(AuthError::InvalidIdToken(
                "refreshed ID Token subject does not match the original"
            ))
        ));
    }

    #[tokio::test]
    async fn state_saved_with_a_malformed_auth_time_keeps_the_other_claims() {
        // Versions up to 0.9.0 accepted ID Tokens without checking auth_time.
        let key = TestKey::rs256();
        let mut legacy = id_token_claims("user-123", None);
        legacy["auth_time"] = serde_json::json!("not-a-number");
        let state = oidc_state(Some(key.sign(legacy)));
        let mut serialized = serde_json::to_value(&state).unwrap();
        serialized
            .as_object_mut()
            .unwrap()
            .remove("id_token_claims")
            .unwrap();

        let mut restored: AuthenticationState = serde_json::from_value(serialized).unwrap();
        let before = serde_json::to_value(&restored).unwrap();
        assert_eq!(
            before["id_token_claims"],
            serde_json::json!({
                "iss": "https://issuer.example/",
                "sub": "user-123",
                "aud": ["native-client"],
            })
        );
        let other_subject = key.sign(id_token_claims("someone-else", None));
        let transport = refresh_transport(Some(&other_subject), &key);
        assert!(matches!(
            refresh_with(&mut restored, "https://issuer.example/", &transport).await,
            Err(AuthError::InvalidIdToken(
                "refreshed ID Token subject does not match the original"
            ))
        ));
        assert_eq!(serde_json::to_value(&restored).unwrap(), before);
    }

    #[tokio::test]
    async fn state_without_a_readable_id_token_records_the_next_validated_one() {
        let key = TestKey::rs256();
        for stored_id_token in [None, Some("old-id".to_owned())] {
            let state = oidc_state(stored_id_token);
            let serialized = serde_json::to_value(&state).unwrap();
            assert!(serialized.get("id_token_claims").is_none());
            let mut restored: AuthenticationState = serde_json::from_value(serialized).unwrap();

            let first = key.sign(id_token_claims("user-123", None));
            let transport = refresh_transport(Some(&first), &key);
            refresh_with(&mut restored, "https://issuer.example/", &transport)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&restored).unwrap()["id_token_claims"]["sub"],
                "user-123"
            );

            let other_subject = key.sign(id_token_claims("someone-else", None));
            let transport = refresh_transport(Some(&other_subject), &key);
            assert!(matches!(
                refresh_with(&mut restored, "https://issuer.example/", &transport).await,
                Err(AuthError::InvalidIdToken(
                    "refreshed ID Token subject does not match the original"
                ))
            ));
        }
    }

    #[test]
    fn state_json_with_claims_round_trips_and_older_readers_ignore_them() {
        // The fields an older version of this library reads.
        #[derive(Deserialize)]
        struct PreviousAuthenticationState {
            token_response: TokenResponse,
            expires_at: Option<u64>,
            #[serde(default)]
            oidc_nonce: Option<String>,
        }

        let key = TestKey::rs256();
        let mut claims = id_token_claims("user-123", Some(1_000));
        claims["aud"] = serde_json::json!(["second", "native-client", "second"]);
        claims["azp"] = serde_json::json!("native-client");
        let state = AuthenticationState::new_with_oidc_nonce(
            TokenResponse {
                access_token: "access".to_owned(),
                token_type: "Bearer".to_owned(),
                expires_in: Some(60),
                refresh_token: Some("refresh".to_owned()),
                scope: None,
                id_token: Some(key.sign(claims)),
                additional_parameters: BTreeMap::new(),
            },
            Some("nonce".to_owned()),
        )
        .unwrap();
        let serialized = serde_json::to_value(&state).unwrap();
        assert_eq!(
            serialized["id_token_claims"],
            serde_json::json!({
                "iss": "https://issuer.example/",
                "sub": "user-123",
                "aud": ["native-client", "second"],
                "azp": "native-client",
                "auth_time": 1_000,
            })
        );

        let restored: AuthenticationState = serde_json::from_value(serialized.clone()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), serialized);

        let previous: PreviousAuthenticationState =
            serde_json::from_value(serialized.clone()).unwrap();
        assert_eq!(previous.token_response.access_token, "access");
        assert_eq!(previous.expires_at, state.expires_at());
        assert_eq!(previous.oidc_nonce.as_deref(), Some("nonce"));
    }

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
