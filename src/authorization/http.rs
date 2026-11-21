use http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use url::Url;

use super::{
    json::UniqueJsonObject,
    state::parse_token_response,
    validation::{RESERVED_REVOCATION_PARAMETERS, RESERVED_TOKEN_PARAMETERS},
    TokenResponse,
};
use crate::{AuthError, CancellationHandle, HttpRequest, HttpTransport, Result};

pub(super) async fn send_token_request(
    token_endpoint: &Url,
    body: Vec<u8>,
    transport: &dyn HttpTransport,
    cancellation: CancellationHandle,
) -> Result<TokenResponse> {
    if token_endpoint
        .query_pairs()
        .any(|(name, _)| RESERVED_TOKEN_PARAMETERS.contains(&name.as_ref()))
    {
        return Err(AuthError::InvalidConfiguration(
            "token endpoint query contains a reserved parameter",
        ));
    }
    let response = send_form_request(token_endpoint, body, transport, cancellation).await?;
    if response.status != StatusCode::OK {
        return Err(oauth_error_or_status(response.status, &response.body));
    }
    parse_token_response(&response.body)
}

pub(super) async fn send_revocation_request(
    revocation_endpoint: &Url,
    body: Vec<u8>,
    transport: &dyn HttpTransport,
    cancellation: CancellationHandle,
) -> Result<()> {
    if revocation_endpoint
        .query_pairs()
        .any(|(name, _)| RESERVED_REVOCATION_PARAMETERS.contains(&name.as_ref()))
    {
        return Err(AuthError::InvalidConfiguration(
            "revocation endpoint query contains a reserved parameter",
        ));
    }
    let response = send_form_request(revocation_endpoint, body, transport, cancellation).await?;
    // RFC 7009 section 2.2: only 200 signals successful revocation (including
    // an already-invalid token), and its body is ignored regardless of content.
    if response.status == StatusCode::OK {
        return Ok(());
    }
    Err(oauth_error_or_status(response.status, &response.body))
}

async fn send_form_request(
    endpoint: &Url,
    body: Vec<u8>,
    transport: &dyn HttpTransport,
    cancellation: CancellationHandle,
) -> Result<crate::HttpResponse> {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    let request = HttpRequest {
        method: Method::POST,
        url: endpoint.clone(),
        headers,
        body,
    };
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(AuthError::Cancelled),
        response = transport.execute(request, cancellation.clone()) => response,
    }
}

fn oauth_error_or_status(status: StatusCode, body: &[u8]) -> AuthError {
    // Preserve transient/unexpected HTTP status (notably 429 and 503) even if
    // its body resembles an OAuth error. Retry policy belongs to the host.
    if matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED) {
        if let Some(error) = parse_oauth_error(body) {
            return error;
        }
    }
    AuthError::HttpStatus(status)
}

fn parse_oauth_error(body: &[u8]) -> Option<AuthError> {
    let UniqueJsonObject(object) = serde_json::from_slice(body).ok()?;
    // A response cannot simultaneously report failure and issue token values.
    if [
        "access_token",
        "token_type",
        "expires_in",
        "refresh_token",
        "scope",
        "id_token",
    ]
    .iter()
    .any(|name| object.contains_key(*name))
    {
        return None;
    }
    let code = object.get("error")?.as_str()?;
    let valid_text = |value: &str| {
        value.bytes().all(|byte| {
            byte == b'!'
                || (b'#'..=b'[').contains(&byte)
                || (b']'..=b'~').contains(&byte)
                || byte == b' '
        })
    };
    if code.is_empty() || !valid_text(code) {
        return None;
    }
    let description = match object.get("error_description") {
        None => None,
        Some(value) => {
            let description = value.as_str()?;
            if !valid_text(description) {
                return None;
            }
            Some(description.to_owned())
        }
    };
    if let Some(value) = object.get("error_uri") {
        let uri = value.as_str()?;
        if !valid_text(uri) || uri.contains(' ') {
            return None;
        }
    }
    Some(AuthError::AuthorizationServer {
        code: code.to_owned(),
        description,
    })
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::authorization::test_support::*;
    use crate::HttpResponse;

    #[tokio::test]
    async fn reserved_token_endpoint_queries_fail_before_network_and_preserve_refresh_state() {
        for name in RESERVED_TOKEN_PARAMETERS {
            for encoded in [
                name.to_string(),
                format!("%{:02X}{}", name.as_bytes()[0], &name[1..]),
            ] {
                let endpoint =
                    Url::parse(&format!("https://issuer.example/token?{encoded}=override"))
                        .unwrap();
                let transport = test_transport(
                    StatusCode::OK,
                    br#"{"access_token":"value","token_type":"Bearer"}"#,
                );
                assert!(matches!(
                    send_token_request(
                        &endpoint,
                        Vec::new(),
                        &transport,
                        CancellationHandle::new()
                    )
                    .await,
                    Err(AuthError::InvalidConfiguration(_))
                ));
                let mut configuration = service_configuration();
                configuration.token_endpoint = endpoint;
                let mut state = authentication_state();
                let before = serde_json::to_value(&state).unwrap();
                assert!(matches!(
                    state
                        .refresh(
                            &configuration,
                            "client",
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
        let endpoint = Url::parse("https://issuer.example/token?tenant=one").unwrap();
        let transport = test_transport(
            StatusCode::OK,
            br#"{"access_token":"value","token_type":"Bearer"}"#,
        );
        send_token_request(&endpoint, Vec::new(), &transport, CancellationHandle::new())
            .await
            .unwrap();
        assert_eq!(
            transport.request.lock().unwrap().as_ref().unwrap().url,
            endpoint
        );
    }

    #[tokio::test]
    async fn token_request_rejects_ambiguous_success_response() {
        for body in [
            br#"{"access_token":"first","access_token":"second","token_type":"Bearer"}"#.as_slice(),
            br#"{"access_token":"value","token_type":"Bearer","error":"invalid_grant"}"#.as_slice(),
        ] {
            let transport = test_transport(StatusCode::OK, body);
            assert!(matches!(
                send_token_request(
                    &Url::parse("https://issuer.example/token").unwrap(),
                    Vec::new(),
                    &transport,
                    CancellationHandle::new(),
                )
                .await,
                Err(AuthError::InvalidTokenResponse(_))
            ));
        }
    }

    #[tokio::test]
    async fn token_and_revocation_errors_require_unique_unambiguous_oauth_objects() {
        let malformed = [
            r#"{"error":"first","error":"second"}"#,
            r#"{"error":"first","err\u006fr":"second"}"#,
            r#"{"error":"invalid_grant","extension":1,"extension":2}"#,
            r#"{"error":"invalid_grant","access_token":"issued"}"#,
            r#"{"error":"invalid_grant","refresh_token":null}"#,
            r#"{"error":""}"#,
            r#"{"error":null}"#,
            r#"{"error":1}"#,
            r#"{"error":"invalid\ngrant"}"#,
            r#"{"error":"invalid_grant","error_description":null}"#,
            r#"{"error":"invalid_grant","error_description":1}"#,
            r#"{"error":"invalid_grant","error_description":"line\nbreak"}"#,
            r#"{"error":"invalid_grant","error_uri":{}}"#,
            r#"{"error":"invalid_grant","error_uri":"not a URI"}"#,
            r#"{"access_token":"issued","token_type":"Bearer"}"#,
            r#"{}"#,
            r#"[]"#,
            r#"null"#,
            r#"{"error":"invalid_grant"} {}"#,
            "not JSON",
        ];
        for status in [StatusCode::BAD_REQUEST, StatusCode::UNAUTHORIZED] {
            for body in malformed {
                let endpoint = Url::parse("https://issuer.example/endpoint").unwrap();
                let transport = test_transport(status, body.as_bytes());
                assert!(
                    matches!(send_token_request(&endpoint, Vec::new(), &transport, CancellationHandle::new()).await, Err(AuthError::HttpStatus(actual)) if actual == status),
                    "accepted token error {body}"
                );
                let transport = test_transport(status, body.as_bytes());
                assert!(
                    matches!(send_revocation_request(&endpoint, Vec::new(), &transport, CancellationHandle::new()).await, Err(AuthError::HttpStatus(actual)) if actual == status),
                    "accepted revocation error {body}"
                );
            }
        }
        for name in ["token_type", "expires_in", "scope", "id_token"] {
            let body = format!("{{\"error\":\"invalid_grant\",\"{name}\":null}}");
            assert!(matches!(
                oauth_error_or_status(StatusCode::BAD_REQUEST, body.as_bytes()),
                AuthError::HttpStatus(StatusCode::BAD_REQUEST)
            ));
        }
    }

    #[tokio::test]
    async fn token_and_revocation_preserve_valid_extension_errors() {
        for (status, code) in [
            (StatusCode::BAD_REQUEST, "unsupported_token_type"),
            (StatusCode::UNAUTHORIZED, "invalid_client"),
            (StatusCode::BAD_REQUEST, "provider_specific_error"),
        ] {
            let body = format!("{{\"error\":\"{code}\",\"error_description\":\"request rejected\",\"error_uri\":\"https://issuer.example/help\",\"extension\":{{\"nested\":[1,true]}}}}");
            let endpoint = Url::parse("https://issuer.example/endpoint").unwrap();
            for revoke in [false, true] {
                let transport = test_transport(status, body.as_bytes());
                let error = if revoke {
                    send_revocation_request(
                        &endpoint,
                        Vec::new(),
                        &transport,
                        CancellationHandle::new(),
                    )
                    .await
                    .unwrap_err()
                } else {
                    send_token_request(&endpoint, Vec::new(), &transport, CancellationHandle::new())
                        .await
                        .err()
                        .unwrap()
                };
                assert!(
                    matches!(error, AuthError::AuthorizationServer { code: actual, description: Some(description) } if actual == code && description == "request rejected")
                );
            }
        }
    }

    #[tokio::test]
    async fn unexpected_statuses_remain_http_errors_even_with_oauth_bodies() {
        let endpoint = Url::parse("https://issuer.example/endpoint").unwrap();
        for status in [
            201, 202, 204, 206, 301, 302, 303, 307, 308, 403, 429, 500, 503,
        ] {
            let status = StatusCode::from_u16(status).unwrap();
            for body in [
                br#"{"error":"temporarily_unavailable"}"#.as_slice(),
                br#"{"access_token":"issued","token_type":"Bearer"}"#,
            ] {
                let transport = test_transport(status, body);
                transport
                    .response
                    .lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .headers
                    .insert(header::RETRY_AFTER, HeaderValue::from_static("120"));
                assert!(
                    matches!(send_token_request(&endpoint, Vec::new(), &transport, CancellationHandle::new()).await, Err(AuthError::HttpStatus(actual)) if actual == status)
                );
                let transport = test_transport(status, body);
                assert!(
                    matches!(send_revocation_request(&endpoint, Vec::new(), &transport, CancellationHandle::new()).await, Err(AuthError::HttpStatus(actual)) if actual == status)
                );
            }
        }
    }

    #[tokio::test]
    async fn rejected_token_error_responses_preserve_refresh_state() {
        for (status, body) in [
            (
                StatusCode::BAD_REQUEST,
                br#"{"error":"invalid_grant","error":"invalid_client"}"#.as_slice(),
            ),
            (
                StatusCode::BAD_REQUEST,
                br#"{"error":"invalid_grant","access_token":"issued"}"#,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                br#"{"error":"temporarily_unavailable"}"#,
            ),
            (
                StatusCode::CREATED,
                br#"{"access_token":"issued","token_type":"Bearer"}"#,
            ),
        ] {
            let mut state = authentication_state();
            let before = serde_json::to_value(&state).unwrap();
            let transport = test_transport(status, body);
            assert!(
                matches!(state.refresh(&service_configuration(), "client", &transport, CancellationHandle::new()).await, Err(AuthError::HttpStatus(actual)) if actual == status)
            );
            assert_eq!(serde_json::to_value(&state).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn token_and_revocation_cancel_even_if_transport_ignores_the_handle() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct BlockingTransport {
            started: tokio::sync::Notify,
            dropped: AtomicBool,
        }
        struct DropSignal<'a>(&'a AtomicBool);
        impl Drop for DropSignal<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        #[async_trait]
        impl HttpTransport for BlockingTransport {
            async fn execute(&self, _: HttpRequest, _: CancellationHandle) -> Result<HttpResponse> {
                let _drop = DropSignal(&self.dropped);
                self.started.notify_one();
                std::future::pending().await
            }
        }
        let endpoint = Url::parse("https://issuer.example/endpoint").unwrap();
        for revoke in [false, true] {
            let transport = BlockingTransport {
                started: tokio::sync::Notify::new(),
                dropped: AtomicBool::new(false),
            };
            let cancellation = CancellationHandle::new();
            let operation = async {
                if revoke {
                    send_revocation_request(&endpoint, Vec::new(), &transport, cancellation.clone())
                        .await
                } else {
                    send_token_request(&endpoint, Vec::new(), &transport, cancellation.clone())
                        .await
                        .map(|_| ())
                }
            };
            tokio::pin!(operation);
            tokio::select! { _ = transport.started.notified() => {}, result = &mut operation => panic!("request should block: {result:?}") }
            cancellation.cancel();
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(2), operation)
                    .await
                    .unwrap(),
                Err(AuthError::Cancelled)
            ));
            assert!(transport.dropped.load(Ordering::SeqCst));
            let transport = test_transport(StatusCode::OK, b"unused");
            if revoke {
                assert!(matches!(
                    send_revocation_request(
                        &endpoint,
                        Vec::new(),
                        &transport,
                        cancellation.clone()
                    )
                    .await,
                    Err(AuthError::Cancelled)
                ));
            } else {
                assert!(matches!(
                    send_token_request(&endpoint, Vec::new(), &transport, cancellation.clone())
                        .await,
                    Err(AuthError::Cancelled)
                ));
            }
            assert!(transport.request.lock().unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn token_requests_surface_http_status_without_an_oauth_error_body() {
        let transport = test_transport(StatusCode::SERVICE_UNAVAILABLE, b"temporarily unavailable");

        assert!(matches!(
            send_token_request(
                &Url::parse("https://issuer.example/token").unwrap(),
                Vec::new(),
                &transport,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::HttpStatus(status)) if status == StatusCode::SERVICE_UNAVAILABLE
        ));
    }
}
