use std::collections::HashSet;

use http::{header, HeaderMap, HeaderValue, Method};
use serde::Serialize;
use serde_json::{Map, Value};
use url::Url;

use super::{
    validation::{validate_bearer_token, validate_redirect_uri},
    ServiceConfiguration,
};
use crate::{AuthError, CancellationHandle, HttpRequest, HttpTransport, Result};

/// A dynamic client registration request for a native public client.
pub struct ClientRegistrationRequest {
    registration_endpoint: Url,
    redirect_uris: Vec<Url>,
    client_name: Option<String>,
    initial_access_token: Option<String>,
}

impl ClientRegistrationRequest {
    /// Creates a request using the configured registration endpoint.
    ///
    /// At least one unique redirect URI is required.
    pub fn new(configuration: &ServiceConfiguration, redirect_uris: Vec<Url>) -> Result<Self> {
        let registration_endpoint =
            configuration
                .registration_endpoint
                .clone()
                .ok_or(AuthError::InvalidConfiguration(
                    "client registration endpoint is not configured",
                ))?;
        if redirect_uris.is_empty() {
            return Err(AuthError::InvalidConfiguration(
                "client registration requires at least one redirect URI",
            ));
        }

        let mut unique_redirect_uris = HashSet::new();
        for redirect_uri in &redirect_uris {
            validate_redirect_uri(redirect_uri)?;
            if !unique_redirect_uris.insert(redirect_uri.as_str()) {
                return Err(AuthError::InvalidConfiguration(
                    "client registration contains a duplicate redirect URI",
                ));
            }
        }

        Ok(Self {
            registration_endpoint,
            redirect_uris,
            client_name: None,
            initial_access_token: None,
        })
    }

    /// Sets the optional display name sent to the registration endpoint.
    pub fn with_client_name(mut self, client_name: impl Into<String>) -> Result<Self> {
        let client_name = client_name.into();
        if client_name.trim().is_empty() {
            return Err(AuthError::InvalidConfiguration(
                "client name must not be empty",
            ));
        }
        self.client_name = Some(client_name);
        Ok(self)
    }

    /// Sets a per-request bearer token for a protected registration endpoint.
    pub fn with_initial_access_token(
        mut self,
        initial_access_token: impl Into<String>,
    ) -> Result<Self> {
        let initial_access_token = initial_access_token.into();
        validate_bearer_token(&initial_access_token)?;
        self.initial_access_token = Some(initial_access_token);
        Ok(self)
    }

    /// Registers the public client and returns its client ID.
    pub async fn register(
        self,
        transport: &dyn HttpTransport,
        cancellation: CancellationHandle,
    ) -> Result<ClientRegistrationResponse> {
        let metadata = ClientRegistrationMetadata {
            application_type: "native",
            redirect_uris: self.redirect_uris.iter().map(Url::as_str).collect(),
            response_types: ["code"],
            grant_types: ["authorization_code", "refresh_token"],
            token_endpoint_auth_method: "none",
            client_name: self.client_name.as_deref(),
        };
        let body = serde_json::to_vec(&metadata)
            .map_err(|_| AuthError::InvalidConfiguration("registration metadata is invalid"))?;

        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if let Some(initial_access_token) = &self.initial_access_token {
            let mut authorization =
                HeaderValue::from_str(&format!("Bearer {initial_access_token}")).map_err(|_| {
                    AuthError::InvalidConfiguration(
                        "initial access token is not a valid bearer token",
                    )
                })?;
            authorization.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, authorization);
        }

        let request = HttpRequest {
            method: Method::POST,
            url: self.registration_endpoint,
            headers,
            body,
        };
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(AuthError::Cancelled),
            response = transport.execute(request, cancellation.clone()) => response,
        }?;
        if !response.status.is_success() {
            if let Ok(value) = serde_json::from_slice::<Value>(&response.body) {
                if let Some(code) = value.get("error").and_then(Value::as_str) {
                    return Err(AuthError::ClientRegistrationServer {
                        code: code.to_owned(),
                        description: value
                            .get("error_description")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    });
                }
            }
            return Err(AuthError::HttpStatus(response.status));
        }

        parse_client_registration_response(&response.body)
    }
}

#[derive(Serialize)]
struct ClientRegistrationMetadata<'a> {
    application_type: &'static str,
    redirect_uris: Vec<&'a str>,
    response_types: [&'static str; 1],
    grant_types: [&'static str; 2],
    token_endpoint_auth_method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_name: Option<&'a str>,
}

/// The client identifier returned by dynamic registration.
pub struct ClientRegistrationResponse {
    client_id: String,
}

impl ClientRegistrationResponse {
    /// Returns the registered public client ID.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
}

fn parse_client_registration_response(body: &[u8]) -> Result<ClientRegistrationResponse> {
    let mut object: Map<String, Value> = serde_json::from_slice(body).map_err(|_| {
        AuthError::InvalidClientRegistrationResponse("response is not a JSON object")
    })?;
    if object.contains_key("client_secret") {
        return Err(AuthError::InvalidClientRegistrationResponse(
            "public client registration response must not include a client_secret",
        ));
    }
    if object.get("application_type").and_then(Value::as_str) != Some("native") {
        return Err(AuthError::InvalidClientRegistrationResponse(
            "application_type is not native",
        ));
    }
    if object
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str)
        != Some("none")
    {
        return Err(AuthError::InvalidClientRegistrationResponse(
            "token endpoint authentication method is not none",
        ));
    }

    let client_id = object
        .remove("client_id")
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|client_id| !client_id.trim().is_empty())
        .ok_or(AuthError::InvalidClientRegistrationResponse(
            "client_id is missing or invalid",
        ))?;
    Ok(ClientRegistrationResponse { client_id })
}

#[cfg(test)]
mod tests {
    use http::StatusCode;

    use super::*;
    use crate::authorization::test_support::*;

    #[tokio::test]
    async fn client_registration_sends_public_native_metadata() {
        let configuration = service_configuration()
            .with_registration_endpoint(Url::parse("https://issuer.example/register").unwrap())
            .unwrap();
        let redirect_uri = Url::parse("com.example.app:/oauth2redirect").unwrap();
        let transport = test_transport(
            StatusCode::CREATED,
            br#"{
                "client_id":"registered-client",
                "application_type":"native",
                "token_endpoint_auth_method":"none"
            }"#,
        );
        let response = ClientRegistrationRequest::new(&configuration, vec![redirect_uri.clone()])
            .unwrap()
            .with_client_name("Example App")
            .unwrap()
            .with_initial_access_token("registration-token")
            .unwrap()
            .register(&transport, CancellationHandle::new())
            .await
            .unwrap();

        assert_eq!(response.client_id(), "registered-client");
        let request = transport.request.lock().unwrap().take().unwrap();
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.url.as_str(), "https://issuer.example/register");
        assert_eq!(
            request.headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let authorization = request.headers.get(header::AUTHORIZATION).unwrap();
        assert_eq!(authorization, "Bearer registration-token");
        assert!(authorization.is_sensitive());

        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["application_type"], "native");
        assert_eq!(body["client_name"], "Example App");
        assert_eq!(
            body["grant_types"],
            serde_json::json!(["authorization_code", "refresh_token"])
        );
        assert_eq!(
            body["redirect_uris"],
            serde_json::json!([redirect_uri.as_str()])
        );
        assert_eq!(body["response_types"], serde_json::json!(["code"]));
        assert_eq!(body["token_endpoint_auth_method"], "none");
        assert!(body.get("client_secret").is_none());
    }

    #[tokio::test]
    async fn client_registration_rejects_responses_that_are_not_public_clients() {
        let configuration = service_configuration()
            .with_registration_endpoint(Url::parse("https://issuer.example/register").unwrap())
            .unwrap();
        let request = || {
            ClientRegistrationRequest::new(
                &configuration,
                vec![Url::parse("com.example.app:/oauth2redirect").unwrap()],
            )
            .unwrap()
        };

        let with_client_secret = test_transport(
            StatusCode::CREATED,
            br#"{
                "client_id":"registered-client",
                "client_secret":"must-not-be-used",
                "application_type":"native",
                "token_endpoint_auth_method":"none"
            }"#,
        );
        assert!(matches!(
            request()
                .register(&with_client_secret, CancellationHandle::new())
                .await,
            Err(AuthError::InvalidClientRegistrationResponse(
                "public client registration response must not include a client_secret"
            ))
        ));

        let wrong_auth_method = test_transport(
            StatusCode::CREATED,
            br#"{
                "client_id":"registered-client",
                "application_type":"native",
                "token_endpoint_auth_method":"client_secret_basic"
            }"#,
        );
        assert!(matches!(
            request()
                .register(&wrong_auth_method, CancellationHandle::new())
                .await,
            Err(AuthError::InvalidClientRegistrationResponse(
                "token endpoint authentication method is not none"
            ))
        ));
    }

    #[tokio::test]
    async fn client_registration_surfaces_server_errors() {
        let configuration = service_configuration()
            .with_registration_endpoint(Url::parse("https://issuer.example/register").unwrap())
            .unwrap();
        let transport = test_transport(
            StatusCode::BAD_REQUEST,
            br#"{
                "error":"invalid_redirect_uri",
                "error_description":"redirect URI is not allowed"
            }"#,
        );
        let result = ClientRegistrationRequest::new(
            &configuration,
            vec![Url::parse("com.example.app:/oauth2redirect").unwrap()],
        )
        .unwrap()
        .register(&transport, CancellationHandle::new())
        .await;

        let request = transport.request.lock().unwrap().take().unwrap();
        assert!(request.headers.get(header::AUTHORIZATION).is_none());
        assert!(matches!(
            result,
            Err(AuthError::ClientRegistrationServer {
                code,
                description: Some(description),
            }) if code == "invalid_redirect_uri" && description == "redirect URI is not allowed"
        ));
    }

    #[test]
    fn client_registration_validates_endpoint_redirects_and_access_tokens() {
        let configuration = service_configuration();
        let redirect_uri = Url::parse("com.example.app:/oauth2redirect").unwrap();
        assert!(matches!(
            configuration
                .clone()
                .with_registration_endpoint(Url::parse("http://issuer.example/register").unwrap()),
            Err(AuthError::InvalidConfiguration(
                "client registration endpoint must use HTTPS without user info or fragments"
            ))
        ));
        assert!(matches!(
            ClientRegistrationRequest::new(&configuration, vec![redirect_uri.clone()]),
            Err(AuthError::InvalidConfiguration(
                "client registration endpoint is not configured"
            ))
        ));

        let configuration = configuration
            .with_registration_endpoint(Url::parse("https://issuer.example/register").unwrap())
            .unwrap();
        assert!(matches!(
            ClientRegistrationRequest::new(&configuration, vec![]),
            Err(AuthError::InvalidConfiguration(
                "client registration requires at least one redirect URI"
            ))
        ));
        assert!(matches!(
            ClientRegistrationRequest::new(
                &configuration,
                vec![redirect_uri.clone(), redirect_uri]
            ),
            Err(AuthError::InvalidConfiguration(
                "client registration contains a duplicate redirect URI"
            ))
        ));
        assert!(matches!(
            ClientRegistrationRequest::new(
                &configuration,
                vec![Url::parse("com.example.app:/oauth2redirect?state=unexpected").unwrap()]
            ),
            Err(AuthError::InvalidConfiguration(
                "redirect URI query contains an authorization response parameter"
            ))
        ));
        assert!(matches!(
            ClientRegistrationRequest::new(
                &configuration,
                vec![Url::parse("com.example.app:/oauth2redirect").unwrap()]
            )
            .unwrap()
            .with_initial_access_token("not a bearer token"),
            Err(AuthError::InvalidConfiguration(
                "initial access token is not a valid bearer token"
            ))
        ));
    }
}
