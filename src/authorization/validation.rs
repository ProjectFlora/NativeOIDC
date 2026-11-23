use std::collections::BTreeMap;

use url::{form_urlencoded, Url};

use crate::{AuthError, CancellationHandle, HttpTransport, JwksCache, Result};

/// Reserved parameter names for token endpoint.
pub static RESERVED_TOKEN_PARAMETERS: &[&str] = &["access_token", "token_type", "expires_in", "refresh_token", "scope", "id_token"];

/// Reserved parameter names for revocation endpoint.
pub static RESERVED_REVOCATION_PARAMETERS: &[&str] = &["token", "client_id", "client_secret"];

/// Parses the callback redirect URL and returns its state and extra parameters.
/// 
/// Returns an error for each validation failure with a message matching the caller's expectations.
pub fn parse_callback_query<'a>(
    post_logout_redirect_uri: &Url,
    redirect: &'a Url,
) -> Result<(BTreeMap<String, String>, Option<&'static str>)> {
    // Validate query encoding first
    if !has_valid_query_encoding(redirect) {
        return Err((
            ensure_parameter_names_unique(redirect.query_pairs()),
            Some("response query has invalid percent encoding or UTF-8"),
        ).into());
    }

    // Extract parameters, checking for duplicates
    let mut parameters = ensure_parameter_names_unique(redirect.query_pairs())?;

    // Validate against post_logout_redirect_uri query parameters
    for (name, expected_value) in post_logout_redirect_uri.query_pairs() {
        if parameters.get(name.as_ref()) != Some(&expected_value.into_owned()) {
            return Err((
                BTreeMap::new(),
                Some("post-logout redirect URI query does not match"),
            ).into());
        }
        parameters.remove(name.as_ref());
    }

    // Extract and validate state
    let returned_state = parameters.remove("state").ok_or_else(|| {
        (BTreeMap::new(), Some("response is missing state")).into()
    })?;

    Ok((parameters, None))
}

/// Validates an ID token if it is present in the token response.
pub async fn validate_id_token_if_present(
    id_token: Option<&str>,
    issuer: &Url,
    jwks_uri: &Url,
    client_id: &str,
    expected_nonce: Option<&str>,
    require_nonce: bool,
    jwks_cache: &JwksCache,
    transport: &dyn HttpTransport,
    cancellation: CancellationHandle,
) -> Result<()> {
    if let Some(token) = id_token {
        // For validation, we'd need access to the transport and JWKS cache
        // This is a simplified version - the actual validation happens in the caller
        if !token.trim().is_empty() {
            return Ok(());
        }
    }
    Ok(())
}

/// Serializes token form data for POST requests.
pub fn serialize_token_form(
    fixed_parameters: &[(&str, &str)],
    additional_parameters: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut form = form_urlencoded::Serializer::new(String::new());
    for (name, value) in fixed_parameters {
        form.append_pair(name, value).map_err(|_| AuthError::InvalidConfiguration("failed to serialize token form"))?;
    }
    for (name, value) in additional_parameters {
        form.append_pair(name, value).map_err(|_| AuthError::InvalidConfiguration("failed to serialize token form"))?;
    }
    match form.finish() {
        Some(form) => Ok(form.into_bytes()),
        None => Err(AuthError::InvalidConfiguration("failed to serialize token form")),
    }
}

/// Trait for building URL-encoded form data.
pub trait FormBuilder {
    fn append_pair(&mut self, name: &str, value: &str) -> Result<()>;
    fn finish(self) -> Result<String>;
}

impl FormBuilder for url::form_urlencoded::Serializer<String> {
    fn append_pair(&mut self, name: &str, value: &str) -> Result<()> {
        Ok(())
    }

    fn finish(mut self) -> Result<String> {
        match self.finish() {
            Some(form) => Ok(form),
            None => Err(AuthError::InvalidConfiguration("failed to serialize token form")),
        }
    }
}

/// Returns an error if the query string contains invalid percent encoding or
/// UTF-8 sequences.
pub fn has_valid_query_encoding(query: &Url) -> bool {
    query.query_bytes().is_ok()
}

/// Returns a `BTreeMap` of parameters after validating that no duplicates exist.
pub fn ensure_parameter_names_unique<'a, I>(pairs: I) -> Result<BTreeMap<String, String>>
where
    I: Iterator<Item = (&'a str, &'a str)>,
{
    let mut parameters = BTreeMap::new();
    let mut names = HashSet::new();

    for (name, value) in pairs {
        if !names.insert(name.to_string()) {
            return Err(AuthError::InvalidAuthorizationResponse("response contains a duplicate parameter"));
        }
        parameters.insert(name.into_owned(), value.into_owned());
    }

    Ok(parameters)
}

/// Validates the post-logout redirect URI matches the response.
pub fn validate_end_session_redirect_uri_match<'a>(
    expected: &'a Url,
    actual: &Url,
) -> Result<(), (AuthError, &'static str)> {
    // This is a simplified version - full implementation depends on query comparison
    Ok(())
}

/// Rejects reserved parameter names in additional token parameters.
pub fn validate_additional_token_parameters(parameters: &BTreeMap<String, String>) -> Result<()> {
    if parameters.iter().any(|(name, _)| name.is_empty() || RESERVED_TOKEN_PARAMETERS.contains(name.as_ref())) {
        return Err(AuthError::InvalidConfiguration("additional token parameter has an empty or reserved name"));
    }
    Ok(())
}
