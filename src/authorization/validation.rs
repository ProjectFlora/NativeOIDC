use std::collections::{BTreeMap, HashSet};

use url::{Host, Url};

use crate::{AuthError, Result};

pub(super) const RESERVED_AUTHORIZATION_PARAMETERS: &[&str] = &[
    "client_id",
    "code_challenge",
    "code_challenge_method",
    "nonce",
    "redirect_uri",
    "response_type",
    "scope",
    "state",
];

pub(super) const RESERVED_TOKEN_PARAMETERS: &[&str] = &[
    "client_assertion",
    "client_assertion_type",
    "client_id",
    "client_secret",
    "code",
    "code_verifier",
    "grant_type",
    "redirect_uri",
    "refresh_token",
    "scope",
];

pub(super) const RESERVED_REVOCATION_PARAMETERS: &[&str] = &[
    "token",
    "token_type_hint",
    "client_id",
    "client_secret",
    "client_assertion",
    "client_assertion_type",
];

const RESERVED_END_SESSION_PARAMETERS: &[&str] = &[
    "client_id",
    "id_token_hint",
    "logout_hint",
    "post_logout_redirect_uri",
    "state",
    "ui_locales",
];

pub(super) fn validate_endpoint(endpoint: &Url) -> Result<()> {
    let is_https = endpoint.scheme() == "https";
    let is_loopback_host = match endpoint.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    let is_loopback_http = endpoint.scheme() == "http" && is_loopback_host;
    if (!is_https && !is_loopback_http)
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(AuthError::InvalidConfiguration(
            "endpoints must use HTTPS or loopback HTTP without user info or fragments",
        ));
    }
    Ok(())
}

pub(super) fn validate_logout_endpoint(endpoint: &Url) -> Result<()> {
    if endpoint.scheme() != "https" || validate_endpoint(endpoint).is_err() {
        return Err(AuthError::InvalidConfiguration(
            "end-session endpoint must use HTTPS without user info or fragments",
        ));
    }
    Ok(())
}

pub(super) fn validate_registration_endpoint(endpoint: &Url) -> Result<()> {
    if endpoint.scheme() != "https" || validate_endpoint(endpoint).is_err() {
        return Err(AuthError::InvalidConfiguration(
            "client registration endpoint must use HTTPS without user info or fragments",
        ));
    }
    Ok(())
}

pub(super) fn validate_revocation_endpoint(endpoint: &Url) -> Result<()> {
    if validate_endpoint(endpoint).is_err() {
        return Err(AuthError::InvalidConfiguration(
            "revocation endpoint must use HTTPS or loopback HTTP without user info or fragments",
        ));
    }
    Ok(())
}

pub(super) fn validate_oidc_endpoint(endpoint: &Url) -> Result<()> {
    if endpoint.scheme() != "https" || validate_endpoint(endpoint).is_err() {
        return Err(AuthError::InvalidConfiguration(
            "JWKS endpoint must use HTTPS without user info or fragments",
        ));
    }
    Ok(())
}

pub(super) fn validate_bearer_token(token: &str) -> Result<()> {
    let mut padding_started = false;
    let mut has_token_character = false;
    for byte in token.bytes() {
        if byte == b'=' {
            padding_started = true;
        } else if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
        {
            if padding_started {
                return Err(AuthError::InvalidConfiguration(
                    "initial access token is not a valid bearer token",
                ));
            }
            has_token_character = true;
        } else {
            return Err(AuthError::InvalidConfiguration(
                "initial access token is not a valid bearer token",
            ));
        }
    }
    if !has_token_character {
        return Err(AuthError::InvalidConfiguration(
            "initial access token is not a valid bearer token",
        ));
    }
    Ok(())
}

pub(super) fn reject_reserved_end_session_parameters(endpoint: &Url) -> Result<()> {
    if endpoint
        .query_pairs()
        .any(|(name, _)| RESERVED_END_SESSION_PARAMETERS.contains(&name.as_ref()))
    {
        return Err(AuthError::InvalidConfiguration(
            "end-session endpoint query contains a reserved parameter",
        ));
    }
    Ok(())
}

pub(super) fn validate_additional_token_parameter(name: &str) -> Result<()> {
    if name.is_empty() || RESERVED_TOKEN_PARAMETERS.contains(&name) {
        return Err(AuthError::InvalidConfiguration(
            "additional token parameter has an empty or reserved name",
        ));
    }
    Ok(())
}

pub(super) fn validate_additional_token_parameters(
    parameters: &BTreeMap<String, String>,
) -> Result<()> {
    for name in parameters.keys() {
        validate_additional_token_parameter(name)?;
    }
    Ok(())
}

/// Checks an OIDC issuer identifier and returns it parsed as a URL.
///
/// The identifier itself is kept and compared as an exact string; the parsed
/// URL is only used to build the discovery URL and for these checks. Strings
/// that `Url::parse` would silently change by stripping whitespace or control
/// characters are rejected, so the identifier and the parsed URL never refer
/// to different text.
pub(super) fn parse_issuer(issuer: &str) -> Result<Url> {
    let invalid = || {
        AuthError::InvalidConfiguration(
            "OIDC issuer must be an HTTPS URL without user info, query, or fragment",
        )
    };
    if issuer
        .chars()
        .any(|c| c.is_ascii_whitespace() || c.is_control())
    {
        return Err(invalid());
    }
    let parsed = Url::parse(issuer).map_err(|_| invalid())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(parsed)
}

pub(super) fn validate_redirect_uri(redirect_uri: &Url) -> Result<()> {
    if redirect_uri.scheme().is_empty()
        || redirect_uri.fragment().is_some()
        || !redirect_uri.username().is_empty()
        || redirect_uri.password().is_some()
    {
        return Err(AuthError::InvalidConfiguration(
            "redirect URI must have a scheme and must not contain credentials or a fragment",
        ));
    }
    if !has_valid_query_encoding(redirect_uri) {
        return Err(AuthError::InvalidConfiguration(
            "redirect URI query has invalid percent encoding or UTF-8",
        ));
    }
    let mut names = HashSet::new();
    if redirect_uri
        .query_pairs()
        .any(|(name, _)| !names.insert(name.into_owned()))
    {
        return Err(AuthError::InvalidConfiguration(
            "redirect URI query contains a duplicate parameter",
        ));
    }
    if redirect_uri.query_pairs().any(|(name, _)| {
        matches!(
            name.as_ref(),
            "code" | "state" | "error" | "error_description" | "error_uri" | "iss"
        )
    }) {
        return Err(AuthError::InvalidConfiguration(
            "redirect URI query contains an authorization response parameter",
        ));
    }
    Ok(())
}

pub(super) fn validate_scope(scope: &str) -> Result<()> {
    if scope.is_empty()
        || !scope.bytes().all(|byte| {
            byte == b'!' || (b'#'..=b'[').contains(&byte) || (b']'..=b'~').contains(&byte)
        })
    {
        return Err(AuthError::InvalidConfiguration(
            "scope values must be non-empty RFC 6749 scope tokens",
        ));
    }
    Ok(())
}

pub(super) fn reject_reserved_endpoint_parameters(endpoint: &Url) -> Result<()> {
    if endpoint
        .query_pairs()
        .any(|(name, _)| RESERVED_AUTHORIZATION_PARAMETERS.contains(&name.as_ref()))
    {
        return Err(AuthError::InvalidConfiguration(
            "authorization endpoint query contains a reserved parameter",
        ));
    }
    Ok(())
}

/// A failure of the callback query checks shared by authorization and
/// end-session redirects. Each caller maps it to its own error variant and message.
pub(super) enum CallbackQueryError {
    /// The raw query has malformed percent encoding or invalid UTF-8.
    InvalidEncoding,
    /// A parameter name appears more than once.
    DuplicateParameter,
    /// The registered redirect URI's fixed query is missing or differs.
    FixedQueryMismatch,
}

/// Validates the callback query and returns its parameters, minus the fixed
/// query parameters of the registered redirect URI.
pub(super) fn parse_callback_query(
    redirect: &Url,
    expected_redirect_uri: &Url,
    error: impl Fn(CallbackQueryError) -> AuthError,
) -> Result<BTreeMap<String, String>> {
    if !has_valid_query_encoding(redirect) {
        return Err(error(CallbackQueryError::InvalidEncoding));
    }

    let mut parameters = BTreeMap::new();
    let mut names = HashSet::new();
    for (name, value) in redirect.query_pairs() {
        if !names.insert(name.to_string()) {
            return Err(error(CallbackQueryError::DuplicateParameter));
        }
        parameters.insert(name.into_owned(), value.into_owned());
    }

    for (name, expected_value) in expected_redirect_uri.query_pairs() {
        if parameters.get(name.as_ref()) != Some(&expected_value.into_owned()) {
            return Err(error(CallbackQueryError::FixedQueryMismatch));
        }
        parameters.remove(name.as_ref());
    }
    Ok(parameters)
}

// url::query_pairs uses lossy decoding. Check the raw query first so malformed
// escapes and invalid UTF-8 cannot silently become literal '%' or U+FFFD.
pub(super) fn has_valid_query_encoding(url: &Url) -> bool {
    let query = url.query().unwrap_or_default().as_bytes();
    let mut decoded = Vec::with_capacity(query.len());
    let mut index = 0;
    while index < query.len() {
        if query[index] == b'%' {
            let Some(pair) = query.get(index + 1..index + 3) else {
                return false;
            };
            let (Some(high), Some(low)) = (
                (pair[0] as char).to_digit(16),
                (pair[1] as char).to_digit(16),
            ) else {
                return false;
            };
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(query[index]);
            index += 1;
        }
    }
    std::str::from_utf8(&decoded).is_ok()
}

pub(super) fn validate_redirect_uri_match(expected: &Url, received: &Url) -> Result<()> {
    if !redirect_uris_match(expected, received) {
        return Err(AuthError::InvalidAuthorizationResponse(
            "redirect URI does not match the request",
        ));
    }
    Ok(())
}

pub(super) fn validate_end_session_redirect_uri_match(
    expected: &Url,
    received: &Url,
) -> Result<()> {
    if !redirect_uris_match(expected, received) {
        return Err(AuthError::InvalidEndSessionResponse(
            "post-logout redirect URI does not match the request",
        ));
    }
    Ok(())
}

fn redirect_uris_match(expected: &Url, received: &Url) -> bool {
    received.fragment().is_none()
        && expected.scheme() == received.scheme()
        && expected.username() == received.username()
        && expected.password() == received.password()
        && expected.host_str() == received.host_str()
        && expected.port() == received.port()
        && expected.path() == received.path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::{test_support::*, AuthorizationRequest, ServiceConfiguration};

    #[test]
    fn revocation_endpoint_accepts_loopback_http_and_rejects_other_http() {
        for endpoint in [
            "https://issuer.example/revoke",
            "http://127.0.0.1:8080/revoke",
            "http://127.1.2.3/revoke",
            "http://[::1]/revoke",
            "http://localhost/revoke",
        ] {
            let endpoint = Url::parse(endpoint).unwrap();
            let result = service_configuration().with_revocation_endpoint(endpoint.clone());
            assert_eq!(result.unwrap().revocation_endpoint(), Some(&endpoint));
        }

        for url in [
            "http://issuer.example/revoke",
            "http://user@127.0.0.1/revoke",
            "http://user:pass@localhost/revoke",
            "http://127.0.0.1/revoke#fragment",
            "ftp://127.0.0.1/revoke",
        ] {
            let endpoint = Url::parse(url).unwrap();
            let result = service_configuration().with_revocation_endpoint(endpoint);
            assert!(
                matches!(result, Err(AuthError::InvalidConfiguration(_))),
                "{url} must be rejected"
            );
        }
    }

    #[test]
    fn issuer_identifier_is_kept_exactly_and_checked() {
        for issuer in [
            "https://issuer.example",
            "https://issuer.example/",
            "https://issuer.example/tenant",
            "https://issuer.example/tenant/",
        ] {
            let configuration = service_configuration()
                .with_oidc_validation_endpoints(
                    issuer,
                    Url::parse("https://issuer.example/keys").unwrap(),
                )
                .unwrap();
            assert_eq!(configuration.issuer(), Some(issuer));
        }

        for issuer in [
            "not a URL",
            "http://issuer.example",
            "https://user@issuer.example",
            "https://issuer.example?",
            "https://issuer.example/?tenant=one",
            "https://issuer.example#",
            " https://issuer.example",
            "https://issuer.example\n",
            "https://issuer.example/ten\tant",
        ] {
            let result = service_configuration().with_oidc_validation_endpoints(
                issuer,
                Url::parse("https://issuer.example/keys").unwrap(),
            );
            assert!(
                matches!(result, Err(AuthError::InvalidConfiguration(_))),
                "{issuer:?} must be rejected"
            );
        }
    }

    #[test]
    fn loopback_http_stays_rejected_for_end_session_and_registration_endpoints() {
        let endpoint = Url::parse("http://127.0.0.1/logout").unwrap();
        assert!(matches!(
            service_configuration().with_end_session_endpoint(endpoint),
            Err(AuthError::InvalidConfiguration(_))
        ));

        let endpoint = Url::parse("http://127.0.0.1/register").unwrap();
        assert!(matches!(
            service_configuration().with_registration_endpoint(endpoint),
            Err(AuthError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn rejects_insecure_remote_endpoints_and_response_parameters_in_redirect_uri() {
        let insecure_endpoint = ServiceConfiguration::new(
            Url::parse("http://issuer.example/authorize").unwrap(),
            Url::parse("https://issuer.example/token").unwrap(),
        );
        assert!(matches!(
            insecure_endpoint,
            Err(AuthError::InvalidConfiguration(_))
        ));

        let configuration = ServiceConfiguration::new(
            Url::parse("https://issuer.example/authorize").unwrap(),
            Url::parse("https://issuer.example/token").unwrap(),
        )
        .unwrap();
        assert!(matches!(
            AuthorizationRequest::new(
                configuration,
                "native-client",
                Url::parse("com.example.app:/callback?state=static").unwrap(),
                vec![],
            ),
            Err(AuthError::InvalidConfiguration(
                "redirect URI query contains an authorization response parameter"
            ))
        ));
    }
}
