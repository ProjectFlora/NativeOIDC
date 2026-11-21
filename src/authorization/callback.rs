//! Shared callback parsing and form building logic for OAuth/OIDC operations.
//!
//! This module provides pure functions that can be called from different layers
//! (core, FFI, Windows) to consolidate duplicate code while preserving error
//! variants and messages specific to each layer.

use std::collections::{BTreeMap, HashSet};

use url::{form_urlencoded, HasQueryData, Url};

use crate::validation::{has_valid_query_encoding, validate_redirect_uri_match};

/// Validates and parses a callback URI query into a map of parameters.
///
/// Performs the following checks:
/// - URL has valid percent encoding and UTF-8
/// - No duplicate parameter names
/// - Redirect URI query matches (except for state, code, error)
///
/// Returns a map of remaining parameters after removing recognized ones.
fn parse_callback_query_base(redirect_uri: &Url, expected_state: impl AsRef<str>) -> Result<BTreeMap<String, String>, ParseError> {
    validate_redirect_uri_match(redirect_uri)?;
    
    if !has_valid_query_encoding(redirect_uri) {
        return Err(ParseError::InvalidEncoding);
    }

    let mut parameters = BTreeMap::new();
    let mut names = HashSet::new();
    for (name, value) in redirect_uri.query_pairs() {
        if !names.insert(name.to_string()) {
            return Err(ParseError::DuplicateParameter);
        }
        parameters.insert(name.into_owned(), value.into_owned());
    }

    // Validate redirect URI query matches (except state, code, error)
    for (name, expected_value) in redirect_uri.query_pairs() {
        if name == "state" || name == "code" || name == "error" {
            continue;
        }
        if parameters.get(name.as_ref()) != Some(&expected_value.into_owned()) {
            return Err(ParseError::RedirectUriMismatch);
        }
        parameters.remove(name.as_ref());
    }

    // Check state
    let returned_state = parameters.remove("state").ok_or(ParseError::MissingState)?;
    if expected_state.as_ref() != returned_state.as_str() {
        return Err(ParseError::StateMismatch);
    }
    parameters.remove("state");

    // Extract code or error
    let code = parameters.remove("code");
    let error = parameters.remove("error");
    if code.is_some() && error.is_some() {
        return Err(ParseError::BothCodeAndError);
    }
    
    Ok(parameters)
}

/// Parses an authorization callback.
pub fn parse_authorization_callback(redirect_uri: &Url, state: impl AsRef<str>) -> Result<(String, BTreeMap<String, String>), ParseError> {
    let mut parameters = parse_callback_query_base(redirect_uri, state)?;
    
    let code = parameters.remove("code").ok_or(ParseError::MissingCode)?;
    if code.is_empty() {
        return Err(ParseError::EmptyCode);
    }
    
    Ok((code, parameters))
}

/// Parses an end-session callback.
pub fn parse_end_session_callback(redirect_uri: &Url, state: impl AsRef<str>) -> Result<(String, BTreeMap<String, String>), ParseError> {
    // EndSessionResponse doesn't expect a code or error
    let mut parameters = BTreeMap::new();
    
    if !has_valid_query_encoding(redirect_uri) {
        return Err(ParseError::InvalidEncoding);
    }

    let mut names = HashSet::new();
    for (name, value) in redirect_uri.query_pairs() {
        if !names.insert(name.to_string()) {
            return Err(ParseError::DuplicateParameter);
        }
        parameters.insert(name.into_owned(), value.into_owned());
    }

    // Validate redirect URI query matches
    let expected_redirect_uri = redirect_uri;
    for (name, expected_value) in expected_redirect_uri.query_pairs() {
        if parameters.get(name.as_ref()) != Some(&expected_value.into_owned()) {
            return Err(ParseError::RedirectUriMismatch);
        }
        parameters.remove(name.as_ref());
    }

    let returned_state = parameters.remove("state").ok_or(ParseError::MissingState)?;
    
    Ok((returned_state, parameters))
}

/// Validates and parses an authorization error from the callback.
pub fn parse_authorization_error(redirect_uri: &Url) -> Result<Option<(String, Option<String>)>, ParseError> {
    let mut parameters = BTreeMap::new();
    
    if !has_valid_query_encoding(redirect_uri) {
        return Err(ParseError::InvalidEncoding);
    }

    let mut names = HashSet::new();
    for (name, value) in redirect_uri.query_pairs() {
        if !names.insert(name.to_string()) {
            return Err(ParseError::DuplicateParameter);
        }
        parameters.insert(name.into_owned(), value.into_owned());
    }

    // EndSessionResponse doesn't expect a code or error
    let error = parameters.remove("error").ok_or(ParseError::MissingError)?;
    
    if error.is_empty() {
        return Err(ParseError::EmptyError);
    }

    Ok(Some((error, parameters.remove("error_description"))))
}

/// Error types returned by callback parsing functions.
#[derive(Debug)]
pub enum ParseError {
    /// The redirect URI has invalid percent encoding or UTF-8
    InvalidEncoding,
    
    /// The response contains a duplicate parameter
    DuplicateParameter,
    
    /// The response query does not match the registered redirect URI
    RedirectUriMismatch,
    
    /// Response is missing state
    MissingState,
    
    /// State mismatch between session and response
    StateMismatch,
    
    /// Both code and error are present in the response
    BothCodeAndError,
    
    /// Missing code in authorization response
    MissingCode,
    
    /// Authorization code must not be empty
    EmptyCode,
    
    /// Missing error in authorization error response
    MissingError,
    
    /// Authorization error must not be empty
    EmptyError,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEncoding => write!(f, "response query has invalid percent encoding or UTF-8"),
            Self::DuplicateParameter => write!(f, "response contains a duplicate parameter"),
            Self::RedirectUriMismatch => write!(f, "redirect URI query does not match"),
            Self::MissingState => write!(f, "response is missing state"),
            Self::StateMismatch => write!(f, "state mismatch"),
            Self::BothCodeAndError => write!(f, "response contains both code and error"),
            Self::MissingCode => write!(f, "missing authorization code"),
            Self::EmptyCode => write!(f, "authorization code must not be empty"),
            Self::MissingError => write!(f, "missing authorization error"),
            Self::EmptyError => write!(f, "authorization error must not be empty"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Helper to map ParseError to AuthError (core authorization layer).
pub fn parse_error_to_auth_error(err: ParseError) -> crate::AuthError {
    match err {
        ParseError::InvalidEncoding => crate::AuthError::InvalidAuthorizationResponse(
            "response query has invalid percent encoding or UTF-8",
        ),
        ParseError::DuplicateParameter => crate::AuthError::InvalidAuthorizationResponse(
            "response contains a duplicate parameter",
        ),
        ParseError::RedirectUriMismatch => crate::AuthError::InvalidAuthorizationResponse(
            "redirect URI query does not match",
        ),
        ParseError::MissingState | ParseError::StateMismatch => crate::AuthError::InvalidAuthorizationResponse("state mismatch"),
        ParseError::BothCodeAndError => crate::AuthError::InvalidAuthorizationResponse(
            "response contains both code and error",
        ),
        ParseError::MissingCode => crate::AuthError::InvalidAuthorizationResponse(
            "missing authorization code",
        ),
        ParseError::EmptyCode => crate::AuthError::InvalidAuthorizationResponse(
            "authorization code must not be empty",
        ),
    }
}

/// Helper to map ParseError to AuthError (end-session layer).
pub fn parse_error_to_auth_end_session_error(err: ParseError) -> crate::AuthError {
    match err {
        ParseError::InvalidEncoding => crate::AuthError::InvalidEndSessionResponse(
            "response query has invalid percent encoding or UTF-8",
        ),
        ParseError::DuplicateParameter => crate::AuthError::InvalidEndSessionResponse(
            "response contains a duplicate parameter",
        ),
        ParseError::RedirectUriMismatch => crate::AuthError::InvalidEndSessionResponse(
            "post-logout redirect URI query does not match",
        ),
        ParseError::MissingState | ParseError::StateMismatch => crate::AuthError::InvalidEndSessionResponse("state mismatch"),
        _ => crate::AuthError::InvalidEndSessionResponse("invalid end-session response"),
    }
}

/// Builds a token exchange request body with fixed parameters plus additional ones.
pub fn build_token_request_body(
    grant_type: &str,
    code_or_refresh_token: &str,
    redirect_uri: &Url,
    client_id: &str,
    code_verifier: Option<&str>,
    additional_parameters: &BTreeMap<String, String>,
) -> Vec<u8> {
    let mut form = form_urlencoded::Serializer::new(String::new());
    
    // Grant type and required parameters
    form.append_pair("grant_type", grant_type);
    
    match grant_type {
        "authorization_code" => {
            form.append_pair("code", code_or_refresh_token);
            form.append_pair("redirect_uri", redirect_uri.as_str());
            form.append_pair("client_id", client_id);
            if let Some(verifier) = code_verifier {
                form.append_pair("code_verifier", verifier);
            }
        },
        "refresh_token" => {
            form.append_pair("refresh_token", code_or_refresh_token);
            form.append_pair("client_id", client_id);
        },
        _ => {}
    };
    
    // Additional parameters
    for (name, value) in additional_parameters {
        form.append_pair(name, value);
    }
    
    form.finish().into_bytes()
}

/// Checks that all parameter names are unique.
pub fn ensure_unique_parameter_names<T>(parameters: &[T]) -> std::result::Result<(), String>
where
    T: AsRef<str>,
{
    let mut names = BTreeSet::new();
    if parameters.iter().any(|parameter| !names.insert(parameter.as_ref())) {
        return Err("parameter names must be unique".to_owned());
    }
    Ok(())
}

