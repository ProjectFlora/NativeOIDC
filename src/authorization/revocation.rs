use url::form_urlencoded;

use super::{http::send_revocation_request, ServiceConfiguration};
use crate::{AuthError, CancellationHandle, HttpTransport, Result};

/// Hint identifying the type of token submitted for revocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevocationTokenType {
    /// An OAuth access token.
    AccessToken,
    /// An OAuth refresh token.
    RefreshToken,
}

impl RevocationTokenType {
    fn as_str(self) -> &'static str {
        match self {
            Self::AccessToken => "access_token",
            Self::RefreshToken => "refresh_token",
        }
    }
}

/// Requests invalidation of an access token or refresh token under RFC 7009.
///
/// The configuration must include a discovered or explicitly configured
/// revocation endpoint. This operation never deletes local authentication
/// state; callers remain responsible for deleting it regardless of the result.
pub async fn revoke_token(
    configuration: &ServiceConfiguration,
    client_id: &str,
    token: &str,
    token_type_hint: Option<RevocationTokenType>,
    transport: &dyn HttpTransport,
    cancellation: CancellationHandle,
) -> Result<()> {
    if client_id.trim().is_empty() {
        return Err(AuthError::InvalidConfiguration(
            "client ID must not be empty",
        ));
    }
    if token.trim().is_empty() {
        return Err(AuthError::InvalidConfiguration(
            "token to revoke must not be empty",
        ));
    }
    let endpoint =
        configuration
            .revocation_endpoint
            .as_ref()
            .ok_or(AuthError::InvalidConfiguration(
                "service configuration has no token revocation endpoint",
            ))?;

    let body = {
        let mut form = form_urlencoded::Serializer::new(String::new());
        form.append_pair("token", token)
            .append_pair("client_id", client_id);
        if let Some(token_type_hint) = token_type_hint {
            form.append_pair("token_type_hint", token_type_hint.as_str());
        }
        form.finish().into_bytes()
    };
    send_revocation_request(endpoint, body, transport, cancellation).await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use http::{header, Method, StatusCode};
    use url::Url;

    use super::*;
    use crate::authorization::{test_support::*, validation::RESERVED_REVOCATION_PARAMETERS};
    use crate::{HttpRequest, HttpResponse};

    #[tokio::test]
    async fn revocation_accepts_only_200_and_ignores_its_body() {
        let configuration = service_configuration()
            .with_revocation_endpoint(
                Url::parse("https://issuer.example/revoke?tenant=one").unwrap(),
            )
            .unwrap();
        for body in [
            b"".as_slice(),
            b"not JSON",
            br#"{"error":"unsupported_token_type"}"#,
            br#"{"error":"one","error":"two"}"#,
        ] {
            let transport = test_transport(StatusCode::OK, body);
            // The same response covers an invalid or already-revoked token;
            // clients cannot and should not distinguish those server outcomes.
            revoke_token(
                &configuration,
                "client",
                "already-invalid-token",
                None,
                &transport,
                CancellationHandle::new(),
            )
            .await
            .unwrap();
            assert_eq!(
                transport
                    .request
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .url
                    .query(),
                Some("tenant=one")
            );
        }
    }

    #[tokio::test]
    async fn revocation_encodes_public_client_form_and_all_hint_options() {
        let configuration = service_configuration()
            .with_revocation_endpoint(Url::parse("https://issuer.example/revoke").unwrap())
            .unwrap();
        for (hint, expected) in [
            (None, None),
            (Some(RevocationTokenType::AccessToken), Some("access_token")),
            (
                Some(RevocationTokenType::RefreshToken),
                Some("refresh_token"),
            ),
        ] {
            let transport = test_transport(StatusCode::OK, b"");
            revoke_token(
                &configuration,
                "client+/=&",
                "token+/=&?",
                hint,
                &transport,
                CancellationHandle::new(),
            )
            .await
            .unwrap();
            let request = transport.request.lock().unwrap().take().unwrap();
            let form: BTreeMap<_, _> = form_urlencoded::parse(&request.body).into_owned().collect();
            assert_eq!(form["client_id"], "client+/=&");
            assert_eq!(form["token"], "token+/=&?");
            assert_eq!(form.get("token_type_hint").map(String::as_str), expected);
            assert_eq!(form.len(), if hint.is_some() { 3 } else { 2 });
            assert!(!request.headers.contains_key(header::AUTHORIZATION));
        }
    }

    #[tokio::test]
    async fn revocation_rejects_reserved_endpoint_query_and_blank_input_before_sending() {
        for name in RESERVED_REVOCATION_PARAMETERS {
            for encoded in [
                name.to_string(),
                format!("%{:02X}{}", name.as_bytes()[0], &name[1..]),
            ] {
                let configuration = service_configuration()
                    .with_revocation_endpoint(
                        Url::parse(&format!("https://issuer.example/revoke?{encoded}=override"))
                            .unwrap(),
                    )
                    .unwrap();
                let transport = test_transport(StatusCode::OK, b"");
                assert!(matches!(
                    revoke_token(
                        &configuration,
                        "client",
                        "token",
                        None,
                        &transport,
                        CancellationHandle::new()
                    )
                    .await,
                    Err(AuthError::InvalidConfiguration(_))
                ));
                assert!(transport.request.lock().unwrap().is_none());
            }
        }
        let configuration = service_configuration()
            .with_revocation_endpoint(Url::parse("https://issuer.example/revoke").unwrap())
            .unwrap();
        for (client, token) in [
            ("", "token"),
            ("  ", "token"),
            ("client", ""),
            ("client", "  "),
        ] {
            let transport = test_transport(StatusCode::OK, b"");
            assert!(matches!(
                revoke_token(
                    &configuration,
                    client,
                    token,
                    None,
                    &transport,
                    CancellationHandle::new()
                )
                .await,
                Err(AuthError::InvalidConfiguration(_))
            ));
            assert!(transport.request.lock().unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn revocation_uses_the_configured_endpoint_and_public_client_form() {
        let configuration = service_configuration()
            .with_revocation_endpoint(Url::parse("https://issuer.example/revoke").unwrap())
            .unwrap();
        let transport = test_transport(StatusCode::OK, b"");

        revoke_token(
            &configuration,
            "native-client",
            "refresh-token",
            Some(RevocationTokenType::RefreshToken),
            &transport,
            CancellationHandle::new(),
        )
        .await
        .unwrap();

        let request = transport.request.lock().unwrap().take().unwrap();
        let form: BTreeMap<_, _> = form_urlencoded::parse(&request.body).into_owned().collect();
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.url.as_str(), "https://issuer.example/revoke");
        assert_eq!(
            request.headers.get(header::CONTENT_TYPE).unwrap(),
            "application/x-www-form-urlencoded"
        );
        assert_eq!(
            form.get("client_id").map(String::as_str),
            Some("native-client")
        );
        assert_eq!(form.get("token").map(String::as_str), Some("refresh-token"));
        assert_eq!(
            form.get("token_type_hint").map(String::as_str),
            Some("refresh_token")
        );
        assert!(!form.contains_key("client_secret"));
    }

    #[tokio::test]
    async fn revocation_sends_to_a_loopback_http_endpoint() {
        let configuration = service_configuration()
            .with_revocation_endpoint(Url::parse("http://127.0.0.1:8080/revoke").unwrap())
            .unwrap();
        let transport = test_transport(StatusCode::OK, b"");

        revoke_token(
            &configuration,
            "native-client",
            "refresh-token",
            Some(RevocationTokenType::RefreshToken),
            &transport,
            CancellationHandle::new(),
        )
        .await
        .unwrap();

        let request = transport.request.lock().unwrap().take().unwrap();
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.url.as_str(), "http://127.0.0.1:8080/revoke");
    }

    #[tokio::test]
    async fn revocation_requires_an_endpoint_and_surfaces_server_errors() {
        let transport = test_transport(
            StatusCode::BAD_REQUEST,
            br#"{"error":"unsupported_token_type","error_description":"not supported"}"#,
        );
        let configuration = service_configuration()
            .with_revocation_endpoint(Url::parse("https://issuer.example/revoke").unwrap())
            .unwrap();
        assert!(matches!(
            revoke_token(
                &configuration,
                "native-client",
                "access-token",
                Some(RevocationTokenType::AccessToken),
                &transport,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::AuthorizationServer { code, .. }) if code == "unsupported_token_type"
        ));

        let transport = test_transport(StatusCode::OK, b"");
        assert!(matches!(
            revoke_token(
                &service_configuration(),
                "native-client",
                "access-token",
                None,
                &transport,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::InvalidConfiguration(
                "service configuration has no token revocation endpoint"
            ))
        ));
        assert!(transport.request.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn revocation_propagates_transport_cancellation() {
        struct CancellationAwareTransport;

        #[async_trait]
        impl HttpTransport for CancellationAwareTransport {
            async fn execute(
                &self,
                _request: HttpRequest,
                cancellation: CancellationHandle,
            ) -> Result<HttpResponse> {
                cancellation.cancelled().await;
                Err(AuthError::Cancelled)
            }
        }

        let configuration = service_configuration()
            .with_revocation_endpoint(Url::parse("https://issuer.example/revoke").unwrap())
            .unwrap();
        let cancellation = CancellationHandle::new();
        cancellation.cancel();
        assert!(matches!(
            revoke_token(
                &configuration,
                "native-client",
                "refresh-token",
                None,
                &CancellationAwareTransport,
                cancellation,
            )
            .await,
            Err(AuthError::Cancelled)
        ));
    }
}
