use std::collections::BTreeMap;

use url::Url;

use super::{
    validation::{
        parse_callback_query, reject_reserved_end_session_parameters,
        validate_end_session_redirect_uri_match, validate_redirect_uri, CallbackQueryError,
    },
    ServiceConfiguration,
};
use crate::{pkce::random_token, AuthError, Result};

/// Parameters for an OpenID Connect RP-Initiated Logout request.
pub struct EndSessionRequest {
    end_session_endpoint: Url,
    client_id: String,
    id_token_hint: Option<String>,
    post_logout_redirect_uri: Option<Url>,
}

impl EndSessionRequest {
    /// Creates a logout request using the configured end-session endpoint.
    pub fn new(configuration: &ServiceConfiguration, client_id: impl Into<String>) -> Result<Self> {
        let client_id = client_id.into();
        if client_id.trim().is_empty() {
            return Err(AuthError::InvalidConfiguration(
                "client ID must not be empty",
            ));
        }

        let end_session_endpoint =
            configuration
                .end_session_endpoint
                .clone()
                .ok_or(AuthError::InvalidConfiguration(
                    "end-session endpoint is not configured",
                ))?;
        reject_reserved_end_session_parameters(&end_session_endpoint)?;

        Ok(Self {
            end_session_endpoint,
            client_id,
            id_token_hint: None,
            post_logout_redirect_uri: None,
        })
    }

    /// Adds an optional ID Token hint to the logout request.
    pub fn with_id_token_hint(mut self, id_token_hint: impl Into<String>) -> Result<Self> {
        let id_token_hint = id_token_hint.into();
        if id_token_hint.trim().is_empty() {
            return Err(AuthError::InvalidConfiguration(
                "ID Token hint must not be empty",
            ));
        }
        self.id_token_hint = Some(id_token_hint);
        Ok(self)
    }

    /// Sets a callback URI; when set, a random state is generated and validated.
    pub fn with_post_logout_redirect_uri(mut self, redirect_uri: Url) -> Result<Self> {
        validate_redirect_uri(&redirect_uri)?;
        self.post_logout_redirect_uri = Some(redirect_uri);
        Ok(self)
    }

    /// Builds the logout URL and returns a session for validating its callback.
    pub fn begin(self) -> Result<EndSessionSession> {
        let state = self
            .post_logout_redirect_uri
            .as_ref()
            .map(|_| random_token())
            .transpose()?;
        let mut end_session_url = self.end_session_endpoint;
        {
            let mut query = end_session_url.query_pairs_mut();
            query.append_pair("client_id", &self.client_id);
            if let Some(id_token_hint) = &self.id_token_hint {
                query.append_pair("id_token_hint", id_token_hint);
            }
            if let Some(redirect_uri) = &self.post_logout_redirect_uri {
                query.append_pair("post_logout_redirect_uri", redirect_uri.as_str());
            }
            if let Some(state) = &state {
                query.append_pair("state", state);
            }
        }

        Ok(EndSessionSession {
            end_session_url,
            post_logout_redirect_uri: self.post_logout_redirect_uri,
            state,
        })
    }
}

/// A prepared logout URL and optional callback-validation state.
pub struct EndSessionSession {
    end_session_url: Url,
    post_logout_redirect_uri: Option<Url>,
    state: Option<String>,
}

impl EndSessionSession {
    /// Returns the URL to open in an external user agent.
    pub fn end_session_url(&self) -> &Url {
        &self.end_session_url
    }

    /// Validates the post-logout redirect and returns its state and extra parameters.
    pub fn parse_redirect(&self, redirect: &Url) -> Result<EndSessionResponse> {
        let expected_redirect_uri =
            self.post_logout_redirect_uri
                .as_ref()
                .ok_or(AuthError::InvalidEndSessionResponse(
                    "no post-logout redirect URI was requested",
                ))?;
        validate_end_session_redirect_uri_match(expected_redirect_uri, redirect)?;
        let mut parameters =
            parse_callback_query(redirect, expected_redirect_uri, |error| match error {
                CallbackQueryError::InvalidEncoding => AuthError::InvalidEndSessionResponse(
                    "response query has invalid percent encoding or UTF-8",
                ),
                CallbackQueryError::DuplicateParameter => {
                    AuthError::InvalidEndSessionResponse("response contains a duplicate parameter")
                }
                CallbackQueryError::FixedQueryMismatch => AuthError::InvalidEndSessionResponse(
                    "post-logout redirect URI query does not match",
                ),
            })?;

        let returned_state =
            parameters
                .remove("state")
                .ok_or(AuthError::InvalidEndSessionResponse(
                    "response is missing state",
                ))?;
        if self.state.as_deref() != Some(returned_state.as_str()) {
            return Err(AuthError::InvalidEndSessionResponse("state mismatch"));
        }

        Ok(EndSessionResponse {
            state: returned_state,
            additional_parameters: parameters,
        })
    }
}

/// A validated end-session callback.
pub struct EndSessionResponse {
    state: String,
    additional_parameters: BTreeMap<String, String>,
}

impl EndSessionResponse {
    /// Returns the validated state value.
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
    use super::*;
    use crate::authorization::test_support::*;

    #[test]
    fn end_session_request_builds_parameters_and_validates_the_callback() {
        let configuration = service_configuration()
            .with_end_session_endpoint(
                Url::parse("https://issuer.example/logout?tenant=example").unwrap(),
            )
            .unwrap();
        let redirect_uri = Url::parse("com.example.app:/logout?source=app").unwrap();
        let session = EndSessionRequest::new(&configuration, "native-client")
            .unwrap()
            .with_id_token_hint("id-token-hint")
            .unwrap()
            .with_post_logout_redirect_uri(redirect_uri)
            .unwrap()
            .begin()
            .unwrap();
        let parameters: BTreeMap<_, _> = session
            .end_session_url()
            .query_pairs()
            .into_owned()
            .collect();
        let state = parameters.get("state").unwrap();

        assert_eq!(
            parameters.get("client_id").map(String::as_str),
            Some("native-client")
        );
        assert_eq!(
            parameters.get("id_token_hint").map(String::as_str),
            Some("id-token-hint")
        );
        assert_eq!(
            parameters
                .get("post_logout_redirect_uri")
                .map(String::as_str),
            Some("com.example.app:/logout?source=app")
        );
        assert_eq!(
            parameters.get("tenant").map(String::as_str),
            Some("example")
        );

        let callback = Url::parse(&format!(
            "com.example.app:/logout?source=app&state={state}&logout=complete"
        ))
        .unwrap();
        let response = session.parse_redirect(&callback).unwrap();
        assert_eq!(response.state(), state);
        assert_eq!(
            response
                .additional_parameters()
                .get("logout")
                .map(String::as_str),
            Some("complete")
        );

        let mismatched_state =
            Url::parse("com.example.app:/logout?source=app&state=wrong").unwrap();
        assert!(matches!(
            session.parse_redirect(&mismatched_state),
            Err(AuthError::InvalidEndSessionResponse("state mismatch"))
        ));
        let missing_state = Url::parse("com.example.app:/logout?source=app").unwrap();
        assert!(matches!(
            session.parse_redirect(&missing_state),
            Err(AuthError::InvalidEndSessionResponse(
                "response is missing state"
            ))
        ));
        let duplicate_state = Url::parse(&format!(
            "com.example.app:/logout?source=app&state={state}&state={state}"
        ))
        .unwrap();
        assert!(matches!(
            session.parse_redirect(&duplicate_state),
            Err(AuthError::InvalidEndSessionResponse(
                "response contains a duplicate parameter"
            ))
        ));
    }

    #[test]
    fn end_session_rejects_missing_or_insecure_endpoints() {
        let configuration = service_configuration();
        assert!(matches!(
            EndSessionRequest::new(&configuration, "native-client"),
            Err(AuthError::InvalidConfiguration(
                "end-session endpoint is not configured"
            ))
        ));

        assert!(matches!(
            configuration
                .clone()
                .with_end_session_endpoint(Url::parse("http://issuer.example/logout").unwrap()),
            Err(AuthError::InvalidConfiguration(
                "end-session endpoint must use HTTPS without user info or fragments"
            ))
        ));

        let endpoint_with_reserved_query = configuration
            .with_end_session_endpoint(
                Url::parse("https://issuer.example/logout?state=attacker").unwrap(),
            )
            .unwrap();
        assert!(matches!(
            EndSessionRequest::new(&endpoint_with_reserved_query, "native-client"),
            Err(AuthError::InvalidConfiguration(
                "end-session endpoint query contains a reserved parameter"
            ))
        ));
    }

    #[test]
    fn logout_callback_rejects_malformed_encoding_and_encoded_duplicate_state() {
        let configuration = service_configuration()
            .with_end_session_endpoint(Url::parse("https://issuer.example/logout").unwrap())
            .unwrap();
        let session = EndSessionRequest::new(&configuration, "client")
            .unwrap()
            .with_post_logout_redirect_uri(Url::parse("com.example.app:/logout").unwrap())
            .unwrap()
            .begin()
            .unwrap();
        let state = session.state.as_deref().unwrap();
        for suffix in ["&extra=%", "&extra=%FF", "&%73tate=duplicate"] {
            let callback =
                Url::parse(&format!("com.example.app:/logout?state={state}{suffix}")).unwrap();
            assert!(matches!(
                session.parse_redirect(&callback),
                Err(AuthError::InvalidEndSessionResponse(_))
            ));
        }
    }
}
