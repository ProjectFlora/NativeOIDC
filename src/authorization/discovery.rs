use http::{header, HeaderMap, HeaderValue, Method};
use serde::Deserialize;
use url::Url;

use super::{
    validation::{parse_issuer, validate_endpoint},
    ServiceConfiguration,
};
use crate::{
    id_token::JwksCache, AuthError, CancellationHandle, HttpRequest, HttpTransport, Result,
};

const OIDC_DISCOVERY_PATH: [&str; 2] = [".well-known", "openid-configuration"];

impl ServiceConfiguration {
    /// Loads and validates OpenID Provider metadata for `issuer`.
    ///
    /// The transport must not follow redirects. Discovery is required when the
    /// issuer and JWKS endpoint are needed for ID Token validation.
    ///
    /// The issuer identifier is kept exactly as given: the metadata `issuer`
    /// and later ID Token `iss` claims must be identical to it, so
    /// `https://issuer.example` and `https://issuer.example/` are different
    /// issuers. Both load `/.well-known/openid-configuration` from the same
    /// URL. Passing a [`Url`] uses its serialization, which always has a path;
    /// pass a string for an issuer without a path.
    pub async fn discover(
        issuer: impl AsRef<str>,
        transport: &dyn HttpTransport,
        cancellation: CancellationHandle,
    ) -> Result<Self> {
        let issuer = issuer.as_ref();
        let discovery_url = oidc_discovery_url(&parse_issuer(issuer)?)?;
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let request = HttpRequest {
            method: Method::GET,
            url: discovery_url,
            headers,
            body: Vec::new(),
        };
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(AuthError::Cancelled),
            response = transport.execute(request, cancellation.clone()) => response,
        }?;
        if !response.status.is_success() {
            return Err(AuthError::HttpStatus(response.status));
        }

        let metadata: DiscoveryMetadata = serde_json::from_slice(&response.body)
            .map_err(|_| AuthError::InvalidDiscoveryMetadata("response is not valid JSON"))?;
        if metadata.issuer != issuer {
            return Err(AuthError::InvalidDiscoveryMetadata(
                "issuer does not match the requested issuer",
            ));
        }

        let authorization_endpoint = parse_discovered_endpoint(&metadata.authorization_endpoint)?;
        let token_endpoint = parse_discovered_endpoint(&metadata.token_endpoint)?;
        let end_session_endpoint = metadata
            .end_session_endpoint
            .as_deref()
            .map(parse_discovered_endpoint)
            .transpose()?;
        let revocation_endpoint = metadata
            .revocation_endpoint
            .as_deref()
            .map(parse_discovered_endpoint)
            .transpose()?;
        let registration_endpoint = metadata
            .registration_endpoint
            .as_deref()
            .map(parse_discovered_endpoint)
            .transpose()?;
        let jwks_uri = metadata
            .jwks_uri
            .as_deref()
            .map(parse_discovered_endpoint)
            .transpose()?;

        Ok(Self {
            authorization_endpoint,
            token_endpoint,
            revocation_endpoint,
            end_session_endpoint,
            registration_endpoint,
            issuer: Some(issuer.to_owned()),
            jwks_uri,
            jwks_cache: JwksCache::default(),
        })
    }
}

#[derive(Deserialize)]
struct DiscoveryMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    revocation_endpoint: Option<String>,
    end_session_endpoint: Option<String>,
    registration_endpoint: Option<String>,
    jwks_uri: Option<String>,
}

fn oidc_discovery_url(issuer: &Url) -> Result<Url> {
    let mut discovery_url = issuer.clone();
    let mut path_segments = discovery_url
        .path_segments_mut()
        .map_err(|_| AuthError::InvalidConfiguration("OIDC issuer URL cannot be a base URL"))?;
    path_segments.pop_if_empty();
    path_segments.extend(OIDC_DISCOVERY_PATH);
    drop(path_segments);
    Ok(discovery_url)
}

fn parse_discovered_endpoint(value: &str) -> Result<Url> {
    let endpoint = Url::parse(value)
        .map_err(|_| AuthError::InvalidDiscoveryMetadata("endpoint is not a valid URL"))?;
    if endpoint.scheme() != "https" || validate_endpoint(&endpoint).is_err() {
        return Err(AuthError::InvalidDiscoveryMetadata(
            "endpoints must use HTTPS without user info or fragments",
        ));
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use http::StatusCode;
    use serde_json::Value;

    use super::*;
    use crate::authorization::{test_support::*, AuthorizationRequest, ClientRegistrationRequest};
    use crate::HttpResponse;

    fn minimal_metadata() -> Value {
        serde_json::json!({
            "issuer": "https://issuer.example/",
            "authorization_endpoint": "https://issuer.example/authorize",
            "token_endpoint": "https://issuer.example/token",
        })
    }

    async fn discover_body(body: &[u8]) -> Result<ServiceConfiguration> {
        ServiceConfiguration::discover(
            Url::parse("https://issuer.example/").unwrap(),
            &discovery_transport(body),
            CancellationHandle::new(),
        )
        .await
    }

    #[tokio::test]
    async fn discovery_rejects_missing_malformed_and_duplicate_known_fields() {
        for name in ["issuer", "authorization_endpoint", "token_endpoint"] {
            let mut metadata = minimal_metadata();
            metadata.as_object_mut().unwrap().remove(name);
            assert!(
                matches!(
                    discover_body(&serde_json::to_vec(&metadata).unwrap()).await,
                    Err(AuthError::InvalidDiscoveryMetadata(_))
                ),
                "accepted missing {name}"
            );
        }
        for name in [
            "issuer",
            "authorization_endpoint",
            "token_endpoint",
            "jwks_uri",
            "revocation_endpoint",
            "end_session_endpoint",
            "registration_endpoint",
        ] {
            for value in [
                serde_json::json!(1),
                serde_json::json!([]),
                serde_json::json!({}),
            ] {
                let mut metadata = minimal_metadata();
                metadata[name] = value;
                assert!(
                    matches!(
                        discover_body(&serde_json::to_vec(&metadata).unwrap()).await,
                        Err(AuthError::InvalidDiscoveryMetadata(_))
                    ),
                    "accepted malformed {name}"
                );
            }
            let metadata = serde_json::to_string(&minimal_metadata()).unwrap();
            let duplicate = format!(
                "{},\"{name}\":\"https://issuer.example/\",\"{name}\":\"https://other.example/\"}}",
                metadata.trim_end_matches('}')
            );
            assert!(
                matches!(
                    discover_body(duplicate.as_bytes()).await,
                    Err(AuthError::InvalidDiscoveryMetadata(_))
                ),
                "accepted duplicate {name}"
            );
        }
        for body in [br#"[]"#.as_slice(), br#"null"#, br#"{} {}"#, br#"{"issuer":"https://issuer.example/","iss\u0075er":"https://other.example/","authorization_endpoint":"https://issuer.example/a","token_endpoint":"https://issuer.example/t"}"#] {
            assert!(matches!(discover_body(body).await, Err(AuthError::InvalidDiscoveryMetadata(_))));
        }
    }

    #[tokio::test]
    async fn discovery_validates_every_endpoint_and_exact_issuer() {
        for name in [
            "authorization_endpoint",
            "token_endpoint",
            "jwks_uri",
            "revocation_endpoint",
            "end_session_endpoint",
            "registration_endpoint",
        ] {
            for value in [
                "",
                "/relative",
                "http://127.0.0.1/token",
                "https://user:pass@issuer.example/token",
                "https://issuer.example/token#fragment",
            ] {
                let mut metadata = minimal_metadata();
                metadata[name] = Value::String(value.to_owned());
                assert!(
                    matches!(
                        discover_body(&serde_json::to_vec(&metadata).unwrap()).await,
                        Err(AuthError::InvalidDiscoveryMetadata(_))
                    ),
                    "accepted {name}={value}"
                );
            }
        }
        for issuer in [
            "https://issuer.example",
            "https://other.example/",
            "https://issuer.example/tenant",
            "https://issuer.example/#fragment",
        ] {
            let mut metadata = minimal_metadata();
            metadata["issuer"] = Value::String(issuer.to_owned());
            assert!(
                matches!(
                    discover_body(&serde_json::to_vec(&metadata).unwrap()).await,
                    Err(AuthError::InvalidDiscoveryMetadata(_))
                ),
                "accepted issuer {issuer}"
            );
        }
    }

    #[tokio::test]
    async fn discovery_compares_issuer_identifiers_as_exact_strings() {
        for (requested, metadata_issuer, accepted) in [
            ("https://issuer.example", "https://issuer.example", true),
            ("https://issuer.example", "https://issuer.example/", false),
            ("https://issuer.example/", "https://issuer.example/", true),
            ("https://issuer.example/", "https://issuer.example", false),
            (
                "https://issuer.example/tenant",
                "https://issuer.example/tenant",
                true,
            ),
            (
                "https://issuer.example/tenant",
                "https://issuer.example/tenant/",
                false,
            ),
            (
                "https://issuer.example/tenant/",
                "https://issuer.example/tenant/",
                true,
            ),
            (
                "https://issuer.example/tenant/",
                "https://issuer.example/tenant",
                false,
            ),
        ] {
            let mut metadata = minimal_metadata();
            metadata["issuer"] = Value::String(metadata_issuer.to_owned());
            let transport = discovery_transport(&serde_json::to_vec(&metadata).unwrap());
            let result =
                ServiceConfiguration::discover(requested, &transport, CancellationHandle::new())
                    .await;
            let expected_path = if requested.contains("/tenant") {
                "https://issuer.example/tenant/.well-known/openid-configuration"
            } else {
                "https://issuer.example/.well-known/openid-configuration"
            };
            assert_eq!(
                transport
                    .request
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .url
                    .as_str(),
                expected_path
            );
            if accepted {
                assert_eq!(
                    result.unwrap().issuer(),
                    Some(requested),
                    "{requested} vs {metadata_issuer}"
                );
            } else {
                assert!(
                    matches!(
                        result,
                        Err(AuthError::InvalidDiscoveryMetadata(
                            "issuer does not match the requested issuer"
                        ))
                    ),
                    "{requested} vs {metadata_issuer} must be rejected"
                );
            }
        }
    }

    #[tokio::test]
    async fn discovery_allows_extensions_and_optional_endpoint_absence_for_oauth() {
        let mut metadata = minimal_metadata();
        metadata["provider_extension"] = serde_json::json!({"nested": [1, true, null]});
        for explicit_null in [false, true] {
            if explicit_null {
                for name in [
                    "jwks_uri",
                    "revocation_endpoint",
                    "end_session_endpoint",
                    "registration_endpoint",
                ] {
                    metadata[name] = Value::Null;
                }
            }
            let configuration = discover_body(&serde_json::to_vec(&metadata).unwrap())
                .await
                .unwrap();
            assert!(configuration.jwks_uri().is_none());
            assert!(configuration.revocation_endpoint().is_none());
            assert!(configuration.end_session_endpoint().is_none());
            assert!(configuration.registration_endpoint().is_none());
            let redirect = Url::parse("com.example.app:/callback").unwrap();
            AuthorizationRequest::new(configuration.clone(), "client", redirect.clone(), vec![])
                .unwrap()
                .begin()
                .unwrap();
            assert!(matches!(
                AuthorizationRequest::new(configuration, "client", redirect, vec!["openid".into()]),
                Err(AuthError::InvalidConfiguration(_))
            ));
        }
    }

    #[tokio::test]
    async fn discovery_rejects_redirect_statuses_and_does_not_cache_metadata() {
        for status in [301, 302, 303, 307, 308] {
            let transport = test_transport(
                StatusCode::from_u16(status).unwrap(),
                &serde_json::to_vec(&minimal_metadata()).unwrap(),
            );
            assert!(
                matches!(ServiceConfiguration::discover(Url::parse("https://issuer.example/").unwrap(), &transport, CancellationHandle::new()).await, Err(AuthError::HttpStatus(actual)) if actual.as_u16() == status)
            );
        }
        let body = serde_json::to_vec(&minimal_metadata()).unwrap();
        let transport = discovery_transport(&body);
        transport
            .response
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .headers
            .insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("max-age=999999999"),
            );
        ServiceConfiguration::discover(
            Url::parse("https://issuer.example/").unwrap(),
            &transport,
            CancellationHandle::new(),
        )
        .await
        .unwrap();
        assert!(transport.request.lock().unwrap().take().is_some());
        let mut updated = minimal_metadata();
        updated["token_endpoint"] = serde_json::json!("https://other.example/new-token");
        *transport.response.lock().unwrap() = Some(HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: serde_json::to_vec(&updated).unwrap(),
        });
        let refreshed = ServiceConfiguration::discover(
            Url::parse("https://issuer.example/").unwrap(),
            &transport,
            CancellationHandle::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            refreshed.token_endpoint().as_str(),
            "https://other.example/new-token"
        );
        assert!(transport.request.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn discovery_uses_issuer_path_and_validates_endpoints() {
        let transport = discovery_transport(
            br#"{
                "issuer":"https://issuer.example/tenant",
                "authorization_endpoint":"https://issuer.example/tenant/authorize",
                "token_endpoint":"https://issuer.example/tenant/token",
                "end_session_endpoint":"https://issuer.example/tenant/logout",
                "registration_endpoint":"https://issuer.example/tenant/register",
                "jwks_uri":"https://issuer.example/tenant/keys"
            }"#,
        );

        let configuration = ServiceConfiguration::discover(
            Url::parse("https://issuer.example/tenant").unwrap(),
            &transport,
            CancellationHandle::new(),
        )
        .await
        .unwrap();
        let request = transport.request.lock().unwrap().take().unwrap();

        assert_eq!(
            request.url.as_str(),
            "https://issuer.example/tenant/.well-known/openid-configuration"
        );
        assert_eq!(request.method, Method::GET);
        assert_eq!(
            request.headers.get(header::ACCEPT).unwrap(),
            "application/json"
        );
        assert_eq!(
            configuration.authorization_endpoint().as_str(),
            "https://issuer.example/tenant/authorize"
        );
        assert_eq!(
            configuration.token_endpoint().as_str(),
            "https://issuer.example/tenant/token"
        );
        assert_eq!(
            configuration.end_session_endpoint().unwrap().as_str(),
            "https://issuer.example/tenant/logout"
        );
        assert_eq!(
            configuration.registration_endpoint().unwrap().as_str(),
            "https://issuer.example/tenant/register"
        );
        assert_eq!(
            configuration.issuer(),
            Some("https://issuer.example/tenant")
        );
        assert_eq!(
            configuration.jwks_uri().unwrap().as_str(),
            "https://issuer.example/tenant/keys"
        );
    }

    #[tokio::test]
    async fn discovery_rejects_mismatched_issuer_and_insecure_endpoints() {
        let mismatched_issuer = discovery_transport(
            br#"{
                "issuer":"https://other.example",
                "authorization_endpoint":"https://issuer.example/authorize",
                "token_endpoint":"https://issuer.example/token"
            }"#,
        );
        assert!(matches!(
            ServiceConfiguration::discover(
                Url::parse("https://issuer.example").unwrap(),
                &mismatched_issuer,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::InvalidDiscoveryMetadata(
                "issuer does not match the requested issuer"
            ))
        ));

        let insecure_endpoint = discovery_transport(
            br#"{
                "issuer":"https://issuer.example/",
                "authorization_endpoint":"http://issuer.example/authorize",
                "token_endpoint":"https://issuer.example/token"
            }"#,
        );
        assert!(matches!(
            ServiceConfiguration::discover(
                Url::parse("https://issuer.example").unwrap(),
                &insecure_endpoint,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::InvalidDiscoveryMetadata(
                "endpoints must use HTTPS without user info or fragments"
            ))
        ));

        let insecure_logout_endpoint = discovery_transport(
            br#"{
                "issuer":"https://issuer.example/",
                "authorization_endpoint":"https://issuer.example/authorize",
                "token_endpoint":"https://issuer.example/token",
                "end_session_endpoint":"http://issuer.example/logout"
            }"#,
        );
        assert!(matches!(
            ServiceConfiguration::discover(
                Url::parse("https://issuer.example").unwrap(),
                &insecure_logout_endpoint,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::InvalidDiscoveryMetadata(
                "endpoints must use HTTPS without user info or fragments"
            ))
        ));

        let loopback_revocation_endpoint = discovery_transport(
            br#"{
                "issuer":"https://issuer.example/",
                "authorization_endpoint":"https://issuer.example/authorize",
                "token_endpoint":"https://issuer.example/token",
                "revocation_endpoint":"http://127.0.0.1/revoke"
            }"#,
        );
        assert!(matches!(
            ServiceConfiguration::discover(
                Url::parse("https://issuer.example").unwrap(),
                &loopback_revocation_endpoint,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::InvalidDiscoveryMetadata(
                "endpoints must use HTTPS without user info or fragments"
            ))
        ));

        let insecure_registration_endpoint = discovery_transport(
            br#"{
                "issuer":"https://issuer.example/",
                "authorization_endpoint":"https://issuer.example/authorize",
                "token_endpoint":"https://issuer.example/token",
                "registration_endpoint":"http://issuer.example/register"
            }"#,
        );
        assert!(matches!(
            ServiceConfiguration::discover(
                Url::parse("https://issuer.example").unwrap(),
                &insecure_registration_endpoint,
                CancellationHandle::new(),
            )
            .await,
            Err(AuthError::InvalidDiscoveryMetadata(
                "endpoints must use HTTPS without user info or fragments"
            ))
        ));
    }

    #[tokio::test]
    async fn discover_and_registration_cancel_even_if_transport_ignores_the_handle() {
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
        let issuer = Url::parse("https://issuer.example/").unwrap();
        let configuration = service_configuration()
            .with_registration_endpoint(Url::parse("https://issuer.example/register").unwrap())
            .unwrap();
        let registration = || {
            ClientRegistrationRequest::new(
                &configuration,
                vec![Url::parse("com.example.app:/oauth2redirect").unwrap()],
            )
            .unwrap()
        };
        for discover in [true, false] {
            let transport = BlockingTransport {
                started: tokio::sync::Notify::new(),
                dropped: AtomicBool::new(false),
            };
            let cancellation = CancellationHandle::new();
            let operation = async {
                if discover {
                    ServiceConfiguration::discover(issuer.clone(), &transport, cancellation.clone())
                        .await
                        .map(|_| ())
                } else {
                    registration()
                        .register(&transport, cancellation.clone())
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
            if discover {
                assert!(matches!(
                    ServiceConfiguration::discover(
                        issuer.clone(),
                        &transport,
                        cancellation.clone()
                    )
                    .await,
                    Err(AuthError::Cancelled)
                ));
            } else {
                assert!(matches!(
                    registration()
                        .register(&transport, cancellation.clone())
                        .await,
                    Err(AuthError::Cancelled)
                ));
            }
            assert!(transport.request.lock().unwrap().is_none());
        }
    }
}
