use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use http::{HeaderMap, StatusCode};
use url::Url;

use super::{AuthenticationState, AuthorizationRequest, ServiceConfiguration, TokenResponse};
use crate::{CancellationHandle, HttpRequest, HttpResponse, HttpTransport, Result};

pub(super) struct TestTransport {
    pub(super) request: Mutex<Option<HttpRequest>>,
    pub(super) response: Mutex<Option<HttpResponse>>,
}

#[async_trait]
impl HttpTransport for TestTransport {
    async fn execute(
        &self,
        request: HttpRequest,
        _cancellation: CancellationHandle,
    ) -> Result<HttpResponse> {
        *self.request.lock().unwrap() = Some(request);
        Ok(self.response.lock().unwrap().take().unwrap())
    }
}

pub(super) fn test_transport(status: StatusCode, body: &[u8]) -> TestTransport {
    TestTransport {
        request: Mutex::new(None),
        response: Mutex::new(Some(HttpResponse {
            status,
            headers: HeaderMap::new(),
            body: body.to_vec(),
        })),
    }
}

pub(super) fn discovery_transport(body: &[u8]) -> TestTransport {
    test_transport(StatusCode::OK, body)
}

pub(super) fn request(scopes: &[&str]) -> AuthorizationRequest {
    let mut configuration = ServiceConfiguration::new(
        Url::parse("https://issuer.example/authorize").unwrap(),
        Url::parse("https://issuer.example/token").unwrap(),
    )
    .unwrap();
    if scopes.contains(&"openid") {
        configuration = configuration
            .with_oidc_validation_endpoints(
                Url::parse("https://issuer.example/").unwrap(),
                Url::parse("https://issuer.example/keys").unwrap(),
            )
            .unwrap();
    }
    AuthorizationRequest::new(
        configuration,
        "native-client",
        Url::parse("com.example.app:/oauth2redirect").unwrap(),
        scopes.iter().map(|scope| (*scope).to_owned()).collect(),
    )
    .unwrap()
}

pub(super) fn authentication_state() -> AuthenticationState {
    AuthenticationState::new_at(
        TokenResponse {
            access_token: "old-access".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: Some(60),
            refresh_token: Some("old-refresh".to_owned()),
            scope: Some("openid profile".to_owned()),
            id_token: Some("old-id".to_owned()),
            additional_parameters: BTreeMap::new(),
        },
        1_000,
    )
    .unwrap()
}

pub(super) fn service_configuration() -> ServiceConfiguration {
    ServiceConfiguration::new(
        Url::parse("https://issuer.example/authorize").unwrap(),
        Url::parse("https://issuer.example/token").unwrap(),
    )
    .unwrap()
}
