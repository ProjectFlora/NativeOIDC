use std::collections::BTreeMap;

use url::Url;

use super::{
    http::{send_token_request, serialize_token_form},
    validation::{
        parse_callback_query, reject_reserved_authorization_endpoint_parameters,
        validate_additional_token_parameter, validate_redirect_uri, validate_redirect_uri_match,
        validate_scope, CallbackQueryError, RESERVED_AUTHORIZATION_PARAMETERS,
    },
    AuthenticationState, ServiceConfiguration, TokenResponse,
};
use crate::{
    id_token::{validate_id_token_if_present, JwksCache},
    pkce::{random_token, PkcePair},
    AuthError, CancellationHandle, HttpTransport, Result,
};

/// Parameters used to create an Authorization Code + PKCE request.
///
/// ```
/// use native_oidc::{AuthorizationRequest, ServiceConfiguration};
/// use url::Url;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let configuration = ServiceConfiguration::new(
///     Url::parse("https://issuer.example/authorize")?,
///     Url::parse("https://issuer.example/token")?,
/// )?;
/// let session = AuthorizationRequest::new(
///     configuration,
///     "native-client",
///     Url::parse("com.example.app:/oauth/callback")?,
///     vec!["profile".to_owned()],
/// )?
/// .begin()?;
/// let authorization_url = session.authorization_url();
/// # let _ = authorization_url;
/// # Ok(())
/// # }
/// ```
pub struct AuthorizationRequest {
    configuration: ServiceConfiguration,
    client_id: String,
    redirect_uri: Url,
    scopes: Vec<String>,
    additional_parameters: BTreeMap<String, String>,
    additional_token_parameters: BTreeMap<String, String>,
}

impl AuthorizationRequest {
    /// Creates an authorization request.
    ///
    /// Requests containing the `openid` scope require issuer and JWKS
    /// configuration so returned ID Tokens can be validated.
    pub fn new(
        configuration: ServiceConfiguration,
        client_id: impl Into<String>,
        redirect_uri: Url,
        scopes: Vec<String>,
    ) -> Result<Self> {
        let client_id = client_id.into();
        if client_id.trim().is_empty() {
            return Err(AuthError::InvalidConfiguration(
                "client ID must not be empty",
            ));
        }

        validate_redirect_uri(&redirect_uri)?;
        for scope in &scopes {
            validate_scope(scope)?;
        }
        if scopes.iter().any(|scope| scope == "openid")
            && (configuration.issuer.is_none() || configuration.jwks_uri.is_none())
        {
            return Err(AuthError::InvalidConfiguration(
                "OIDC issuer and JWKS endpoint are required to validate ID Tokens",
            ));
        }

        Ok(Self {
            configuration,
            client_id,
            redirect_uri,
            scopes,
            additional_parameters: BTreeMap::new(),
            additional_token_parameters: BTreeMap::new(),
        })
    }

    /// Adds a provider-specific authorization request parameter.
    pub fn with_additional_parameter(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self> {
        let name = name.into();
        if name.is_empty() || RESERVED_AUTHORIZATION_PARAMETERS.contains(&name.as_str()) {
            return Err(AuthError::InvalidConfiguration(
                "additional parameter has an empty or reserved name",
            ));
        }

        self.additional_parameters.insert(name, value.into());
        Ok(self)
    }

    /// Adds a provider-specific parameter to the token exchange request.
    pub fn with_additional_token_parameter(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self> {
        let name = name.into();
        validate_additional_token_parameter(&name)?;
        self.additional_token_parameters.insert(name, value.into());
        Ok(self)
    }

    /// Generates state, an optional OIDC nonce, and an S256 PKCE challenge.
    pub fn begin(self) -> Result<AuthorizationSession> {
        let mut authorization_url = self.configuration.authorization_endpoint.clone();
        reject_reserved_authorization_endpoint_parameters(&authorization_url)?;

        let state = random_token()?;
        let nonce = if self.scopes.iter().any(|scope| scope == "openid") {
            Some(random_token()?)
        } else {
            None
        };
        let pkce = PkcePair::generate()?;

        {
            let mut query = authorization_url.query_pairs_mut();
            query.append_pair("response_type", "code");
            query.append_pair("client_id", &self.client_id);
            query.append_pair("redirect_uri", self.redirect_uri.as_str());
            if !self.scopes.is_empty() {
                query.append_pair("scope", &self.scopes.join(" "));
            }
            query.append_pair("state", &state);
            if let Some(nonce) = &nonce {
                query.append_pair("nonce", nonce);
            }
            query.append_pair("code_challenge", pkce.challenge());
            query.append_pair("code_challenge_method", "S256");
            for (name, value) in self.additional_parameters {
                query.append_pair(&name, &value);
            }
        }

        Ok(AuthorizationSession {
            authorization_url,
            redirect_uri: self.redirect_uri,
            token_endpoint: self.configuration.token_endpoint,
            oidc_issuer: self.configuration.issuer,
            jwks_uri: self.configuration.jwks_uri,
            jwks_cache: self.configuration.jwks_cache,
            client_id: self.client_id,
            additional_token_parameters: self.additional_token_parameters,
            state,
            nonce,
            pkce,
        })
    }
}

/// A prepared authorization request and the session data needed to finish it.
pub struct AuthorizationSession {
    authorization_url: Url,
    redirect_uri: Url,
    token_endpoint: Url,
    oidc_issuer: Option<String>,
    jwks_uri: Option<Url>,
    jwks_cache: JwksCache,
    client_id: String,
    additional_token_parameters: BTreeMap<String, String>,
    state: String,
    nonce: Option<String>,
    pkce: PkcePair,
}

impl AuthorizationSession {
    /// Returns the URL to open in an external user agent.
    pub fn authorization_url(&self) -> &Url {
        &self.authorization_url
    }

    /// Returns the generated OIDC nonce, or `None` when `openid` was not requested.
    pub fn nonce(&self) -> Option<&str> {
        self.nonce.as_deref()
    }

    /// Creates authentication state while retaining this session's OIDC nonce.
    ///
    /// Prefer this method over [`AuthenticationState::new`] after a successful
    /// code exchange so the nonce remains available during later refreshes.
    pub fn authentication_state(
        &self,
        token_response: TokenResponse,
    ) -> Result<AuthenticationState> {
        AuthenticationState::new_with_oidc_nonce(token_response, self.nonce.clone())
    }

    /// Validates a callback URL and extracts its authorization response.
    ///
    /// The redirect URI and state must match this session. OAuth error
    /// responses are returned as [`AuthError::AuthorizationServer`].
    pub fn parse_redirect(&self, redirect: &Url) -> Result<AuthorizationResponse> {
        validate_redirect_uri_match(&self.redirect_uri, redirect)?;
        let mut parameters =
            parse_callback_query(redirect, &self.redirect_uri, |error| match error {
                CallbackQueryError::InvalidEncoding => AuthError::InvalidAuthorizationResponse(
                    "response query has invalid percent encoding or UTF-8",
                ),
                CallbackQueryError::DuplicateParameter => AuthError::InvalidAuthorizationResponse(
                    "response contains a duplicate parameter",
                ),
                CallbackQueryError::FixedQueryMismatch => {
                    AuthError::InvalidAuthorizationResponse("redirect URI query does not match")
                }
            })?;

        if parameters.get("state").map(String::as_str) != Some(self.state.as_str()) {
            return Err(AuthError::InvalidAuthorizationResponse("state mismatch"));
        }
        parameters.remove("state");

        let code = parameters.remove("code");
        let error = parameters.remove("error");
        if code.is_some() && error.is_some() {
            return Err(AuthError::InvalidAuthorizationResponse(
                "response contains both code and error",
            ));
        }
        if let Some(code) = code {
            if code.is_empty() {
                return Err(AuthError::InvalidAuthorizationResponse(
                    "authorization code must not be empty",
                ));
            }
            return Ok(AuthorizationResponse {
                code,
                state: self.state.clone(),
                additional_parameters: parameters,
            });
        }
        if let Some(code) = error {
            if code.is_empty() {
                return Err(AuthError::InvalidAuthorizationResponse(
                    "authorization error must not be empty",
                ));
            }
            return Err(AuthError::AuthorizationServer {
                code,
                description: parameters.remove("error_description"),
            });
        }

        Err(AuthError::InvalidAuthorizationResponse(
            "response contains neither code nor error",
        ))
    }

    /// Exchanges a validated authorization code and verifies any returned ID Token.
    pub async fn exchange_code(
        &self,
        transport: &dyn HttpTransport,
        response: &AuthorizationResponse,
        cancellation: CancellationHandle,
    ) -> Result<TokenResponse> {
        if response.state != self.state {
            return Err(AuthError::InvalidAuthorizationResponse(
                "authorization response belongs to a different session",
            ));
        }
        if self.nonce.is_some() && (self.oidc_issuer.is_none() || self.jwks_uri.is_none()) {
            return Err(AuthError::InvalidConfiguration(
                "OIDC issuer and JWKS endpoint are required to validate ID Tokens",
            ));
        }

        let body = serialize_token_form(
            &[
                ("grant_type", "authorization_code"),
                ("code", &response.code),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("client_id", &self.client_id),
                ("code_verifier", self.pkce.verifier()),
            ],
            &self.additional_token_parameters,
        );
        let token_response =
            send_token_request(&self.token_endpoint, body, transport, cancellation.clone()).await?;
        if self.nonce.is_some() && token_response.id_token.is_none() {
            return Err(AuthError::InvalidIdToken(
                "openid token response is missing an ID Token",
            ));
        }
        validate_id_token_if_present(
            token_response.id_token.as_deref(),
            self.oidc_issuer.as_deref(),
            self.jwks_uri.as_ref(),
            &self.client_id,
            self.nonce.as_deref(),
            self.nonce.is_some(),
            &self.jwks_cache,
            transport,
            cancellation,
        )
        .await?;
        Ok(token_response)
    }
}

/// A validated authorization callback containing an authorization code.
pub struct AuthorizationResponse {
    code: String,
    state: String,
    additional_parameters: BTreeMap<String, String>,
}

impl AuthorizationResponse {
    /// Returns the authorization code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the state matched against the originating session.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Returns unrecognized response parameters without interpreting them.
    pub fn additional_parameters(&self) -> &BTreeMap<String, String> {
        &self.additional_parameters
    }
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use http::{Method, StatusCode};
    use sha2::{Digest, Sha256};

    use url::form_urlencoded;

    use super::*;
    use crate::authorization::{test_support::*, validation::RESERVED_TOKEN_PARAMETERS};

    #[test]
    fn openid_authorization_requires_validation_endpoints_before_browser_launch() {
        assert!(matches!(
            AuthorizationRequest::new(
                service_configuration(),
                "native-client",
                Url::parse("com.example.app:/oauth2redirect").unwrap(),
                vec!["openid".to_owned()],
            ),
            Err(AuthError::InvalidConfiguration(
                "OIDC issuer and JWKS endpoint are required to validate ID Tokens"
            ))
        ));
    }

    #[test]
    fn authorization_request_uses_code_and_s256_pkce() {
        let session = request(&["openid", "profile"]).begin().unwrap();
        let parameters: BTreeMap<_, _> = session
            .authorization_url()
            .query_pairs()
            .into_owned()
            .collect();

        assert_eq!(
            parameters.get("response_type").map(String::as_str),
            Some("code")
        );
        assert_eq!(
            parameters.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(parameters.get("nonce").map(String::as_str), session.nonce());
        assert_eq!(
            parameters.get("scope").map(String::as_str),
            Some("openid profile")
        );
        assert_eq!(
            parameters.get("redirect_uri").map(String::as_str),
            Some("com.example.app:/oauth2redirect")
        );
    }

    #[test]
    fn authorization_response_requires_matching_redirect_and_state() {
        let session = request(&["openid"]).begin().unwrap();
        let state = &session.state;
        let redirect = Url::parse(&format!(
            "com.example.app:/oauth2redirect?code=auth-code&state={state}&provider_hint=tenant"
        ))
        .unwrap();

        let response = session.parse_redirect(&redirect).unwrap();
        assert_eq!(response.code(), "auth-code");
        assert_eq!(
            response
                .additional_parameters
                .get("provider_hint")
                .map(String::as_str),
            Some("tenant")
        );
    }

    #[test]
    fn authorization_response_rejects_mismatched_state_and_redirect_uri() {
        let session = request(&["openid"]).begin().unwrap();
        let wrong_state =
            Url::parse("com.example.app:/oauth2redirect?code=auth-code&state=wrong").unwrap();
        assert!(matches!(
            session.parse_redirect(&wrong_state),
            Err(AuthError::InvalidAuthorizationResponse("state mismatch"))
        ));

        let wrong_path =
            Url::parse("com.example.app:/different?code=auth-code&state=state").unwrap();
        assert!(matches!(
            session.parse_redirect(&wrong_path),
            Err(AuthError::InvalidAuthorizationResponse(
                "redirect URI does not match the request"
            ))
        ));
    }

    #[test]
    fn callback_rejects_empty_duplicate_and_malformed_parameters() {
        let session = request(&[]).begin().unwrap();
        for query in [
            "code=&state=STATE",
            "error=&state=STATE",
            "state=STATE",
            "code=value",
            "code=value&state=",
            "code=value&state=STATE&%73tate=STATE",
            "code=value&state=STATE&extension=1&extension=2",
            "code=value&state=STATE&error=",
            "error=denied&state=wrong",
            "code=%&state=STATE",
            "code=%2&state=STATE",
            "code=%GG&state=STATE",
            "code=%FF&state=STATE",
            "code=%C0%AF&state=STATE",
            "code=value&state=STATE&ext=%ED%A0%80",
            "code=value&state=STATE&%FF=extension",
        ] {
            let callback = Url::parse(&format!(
                "com.example.app:/oauth2redirect?{}",
                query.replace("STATE", session.state.as_str())
            ))
            .unwrap();
            assert!(
                matches!(
                    session.parse_redirect(&callback),
                    Err(AuthError::InvalidAuthorizationResponse(_))
                ),
                "accepted {query}"
            );
        }
        let callback = Url::parse(&format!(
            "com.example.app:/oauth2redirect?code=a%2Bb%25&state={}&ext=%E6%97%A5%E6%9C%AC+text",
            session.state.as_str()
        ))
        .unwrap();
        let response = session.parse_redirect(&callback).unwrap();
        assert_eq!(response.code(), "a+b%");
        assert_eq!(response.additional_parameters["ext"], "日本 text");
    }

    #[test]
    fn callback_requires_registered_components_and_fixed_query() {
        let configuration = service_configuration();
        let session = AuthorizationRequest::new(
            configuration,
            "client",
            Url::parse("https://app.example:8443/callback?tenant=one").unwrap(),
            vec![],
        )
        .unwrap()
        .begin()
        .unwrap();
        for target in [
            "http://app.example:8443/callback?tenant=one",
            "https://other.example:8443/callback?tenant=one",
            "https://app.example/callback?tenant=one",
            "https://app.example:8443/other?tenant=one",
            "https://app.example:8443/callback?tenant=two",
            "https://app.example:8443/callback?other=one",
            "https://user@app.example:8443/callback?tenant=one",
        ] {
            let callback = Url::parse(&format!(
                "{target}&code=value&state={}",
                session.state.as_str()
            ))
            .unwrap();
            assert!(
                matches!(
                    session.parse_redirect(&callback),
                    Err(AuthError::InvalidAuthorizationResponse(_))
                ),
                "accepted {target}"
            );
        }
        let valid = Url::parse(&format!(
            "https://app.example:8443/callback?state={}&tenant=one&code=value",
            session.state.as_str()
        ))
        .unwrap();
        session.parse_redirect(&valid).unwrap();
        let mut fragment = valid;
        fragment.set_fragment(Some("unexpected"));
        assert!(matches!(
            session.parse_redirect(&fragment),
            Err(AuthError::InvalidAuthorizationResponse(_))
        ));
        for query in [
            "tenant=1&tenant=1",
            "tenant=%FF",
            "tenant=%",
            "%73tate=fixed",
        ] {
            assert!(matches!(
                AuthorizationRequest::new(
                    service_configuration(),
                    "client",
                    Url::parse(&format!("com.example.app:/callback?{query}")).unwrap(),
                    vec![]
                ),
                Err(AuthError::InvalidConfiguration(_))
            ));
        }
    }

    #[test]
    fn rejects_duplicate_and_conflicting_authorization_parameters() {
        let session = request(&[]).begin().unwrap();
        let state = &session.state;
        let duplicate = Url::parse(&format!(
            "com.example.app:/oauth2redirect?code=one&code=two&state={state}"
        ))
        .unwrap();
        assert!(matches!(
            session.parse_redirect(&duplicate),
            Err(AuthError::InvalidAuthorizationResponse(
                "response contains a duplicate parameter"
            ))
        ));

        let conflict = Url::parse(&format!(
            "com.example.app:/oauth2redirect?code=one&error=denied&state={state}"
        ))
        .unwrap();
        assert!(matches!(
            session.parse_redirect(&conflict),
            Err(AuthError::InvalidAuthorizationResponse(
                "response contains both code and error"
            ))
        ));
    }

    #[test]
    fn additional_parameters_cannot_override_protocol_fields() {
        let result = request(&[]).with_additional_parameter("state", "attacker-controlled");
        assert!(matches!(
            result,
            Err(AuthError::InvalidConfiguration(
                "additional parameter has an empty or reserved name"
            ))
        ));
    }

    #[test]
    fn all_reserved_names_are_rejected_in_builders_and_authorization_endpoint_query() {
        for name in RESERVED_AUTHORIZATION_PARAMETERS
            .iter()
            .copied()
            .chain(std::iter::once(""))
        {
            assert!(matches!(
                request(&[]).with_additional_parameter(name, "override"),
                Err(AuthError::InvalidConfiguration(_))
            ));
            if name.is_empty() {
                continue;
            }
            for encoded in [
                name.to_owned(),
                format!("%{:02X}{}", name.as_bytes()[0], &name[1..]),
            ] {
                let configuration = ServiceConfiguration::new(
                    Url::parse(&format!(
                        "https://issuer.example/authorize?{encoded}=override"
                    ))
                    .unwrap(),
                    Url::parse("https://issuer.example/token").unwrap(),
                )
                .unwrap();
                assert!(matches!(
                    AuthorizationRequest::new(
                        configuration,
                        "client",
                        Url::parse("com.example.app:/callback").unwrap(),
                        vec![]
                    )
                    .unwrap()
                    .begin(),
                    Err(AuthError::InvalidConfiguration(_))
                ));
            }
        }
        for name in RESERVED_TOKEN_PARAMETERS
            .iter()
            .copied()
            .chain(std::iter::once(""))
        {
            assert!(matches!(
                request(&[]).with_additional_token_parameter(name, "override"),
                Err(AuthError::InvalidConfiguration(_))
            ));
        }
        // Names are case-sensitive, and percent signs in caller-supplied names
        // are serialized literally rather than interpreted as escapes.
        let session = request(&[])
            .with_additional_parameter("STATE", "extension")
            .unwrap()
            .with_additional_parameter("st%61te", "literal")
            .unwrap()
            .begin()
            .unwrap();
        let parameters: BTreeMap<_, _> = session
            .authorization_url()
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(parameters["state"], session.state.as_str());
        assert_eq!(parameters["STATE"], "extension");
        assert_eq!(parameters["st%61te"], "literal");
    }

    #[test]
    fn additional_token_parameters_cannot_override_protocol_or_client_fields() {
        for name in ["grant_type", "refresh_token", "client_id", "client_secret"] {
            let result = request(&[]).with_additional_token_parameter(name, "caller-value");
            assert!(matches!(
                result,
                Err(AuthError::InvalidConfiguration(
                    "additional token parameter has an empty or reserved name"
                ))
            ));
        }
    }

    #[tokio::test]
    async fn code_exchange_sends_verifier_and_preserves_unknown_token_fields() {
        let session = request(&[])
            .with_additional_token_parameter("audience", "https://api.example")
            .unwrap()
            .begin()
            .unwrap();
        let redirect = Url::parse(&format!(
            "com.example.app:/oauth2redirect?code=auth-code&state={}",
            session.state
        ))
        .unwrap();
        let authorization_response = session.parse_redirect(&redirect).unwrap();
        let transport = test_transport(
            StatusCode::OK,
            br#"{
                "access_token":"access",
                "token_type":"Bearer",
                "expires_in":3600,
                "refresh_token":"refresh",
                "provider_data":{"key":"value"}
            }"#,
        );

        let token_response = session
            .exchange_code(
                &transport,
                &authorization_response,
                CancellationHandle::new(),
            )
            .await
            .unwrap();
        let sent_request = transport.request.lock().unwrap().take().unwrap();
        let form: BTreeMap<_, _> = form_urlencoded::parse(&sent_request.body)
            .into_owned()
            .collect();
        let challenge = session
            .authorization_url()
            .query_pairs()
            .find(|(name, _)| name == "code_challenge")
            .unwrap()
            .1;
        let verifier = form.get("code_verifier").unwrap();

        assert_eq!(sent_request.method, Method::POST);
        assert_eq!(sent_request.url.as_str(), "https://issuer.example/token");
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("authorization_code")
        );
        assert_eq!(form.get("code").map(String::as_str), Some("auth-code"));
        assert_eq!(
            form.get("audience").map(String::as_str),
            Some("https://api.example")
        );
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            challenge
        );
        assert_eq!(token_response.access_token, "access");
        assert_eq!(token_response.id_token, None);
        assert_eq!(
            token_response.additional_parameters["provider_data"]["key"],
            "value"
        );
    }

    #[tokio::test]
    async fn openid_code_exchange_rejects_missing_or_malformed_id_tokens() {
        for (body, expected_error) in [
            (
                br#"{"access_token":"access","token_type":"Bearer"}"#.as_slice(),
                "openid token response is missing an ID Token",
            ),
            (
                br#"{"access_token":"access","token_type":"Bearer","id_token":"not.a.jwt"}"#
                    .as_slice(),
                "JWT header is malformed or unsupported",
            ),
        ] {
            let session = request(&["openid"]).begin().unwrap();
            let redirect = Url::parse(&format!(
                "com.example.app:/oauth2redirect?code=auth-code&state={}",
                session.state
            ))
            .unwrap();
            let authorization_response = session.parse_redirect(&redirect).unwrap();
            let transport = test_transport(StatusCode::OK, body);

            assert!(matches!(
                session
                    .exchange_code(
                        &transport,
                        &authorization_response,
                        CancellationHandle::new(),
                    )
                    .await,
                Err(AuthError::InvalidIdToken(message)) if message == expected_error
            ));
        }
    }

    #[tokio::test]
    async fn code_exchange_surfaces_oauth_errors() {
        let session = request(&[]).begin().unwrap();
        let redirect = Url::parse(&format!(
            "com.example.app:/oauth2redirect?code=auth-code&state={}",
            session.state
        ))
        .unwrap();
        let authorization_response = session.parse_redirect(&redirect).unwrap();
        let transport = test_transport(
            StatusCode::BAD_REQUEST,
            br#"{"error":"invalid_grant","error_description":"expired code"}"#,
        );

        assert!(matches!(
            session
                .exchange_code(
                    &transport,
                    &authorization_response,
                    CancellationHandle::new()
                )
                .await,
            Err(AuthError::AuthorizationServer { code, .. }) if code == "invalid_grant"
        ));
    }

    #[tokio::test]
    async fn code_exchange_rejects_response_from_another_session() {
        let first_session = request(&[]).begin().unwrap();
        let second_session = request(&[]).begin().unwrap();
        let redirect = Url::parse(&format!(
            "com.example.app:/oauth2redirect?code=auth-code&state={}",
            first_session.state
        ))
        .unwrap();
        let authorization_response = first_session.parse_redirect(&redirect).unwrap();
        let transport = test_transport(
            StatusCode::OK,
            br#"{"access_token":"access","token_type":"Bearer"}"#,
        );

        assert!(matches!(
            second_session
                .exchange_code(
                    &transport,
                    &authorization_response,
                    CancellationHandle::new()
                )
                .await,
            Err(AuthError::InvalidAuthorizationResponse(
                "authorization response belongs to a different session"
            ))
        ));
        assert!(transport.request.lock().unwrap().is_none());
    }
}
