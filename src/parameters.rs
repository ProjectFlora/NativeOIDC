//! Core function to check that parameter names are unique.
//! This is extracted from two existing copies in `ffi.rs` and `windows/data.rs`.

use std::collections::BTreeSet;

/// Check that all parameter names in an iterator are unique.
pub(crate) fn are_parameter_names_unique<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    names.all(|name| seen.insert(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_unique_names_pass() {
        assert!(are_parameter_names_unique([
            "scope",
            "redirect_uri",
            "state",
            "code_verifier",
            "client_id",
        ].iter()))
    }

    #[test]
    fn first_duplicate_fails() {
        assert!(!are_parameter_names_unique([
            "scope",
            "redirect_uri",
            "scope",  // duplicate
            "state",
            "code_verifier",
        ].iter()))
    }

    #[test]
    fn last_duplicate_fails() {
        assert!(!are_parameter_names_unique([
            "scope",
            "redirect_uri",
            "state",
            "code_verifier",  // duplicate of... no wait, this is unique
            "scope",  // duplicate
        ].iter()))
    }

    #[test]
    fn empty_iterator_passes() {
        assert!(are_parameter_names_unique([].iter()))
    }

    #[test]
    fn single_name_passes() {
        assert!(are_parameter_names_unique(["scope"].iter()))
    }

    #[test]
    fn two_duplicate_names_fail() {
        let names = vec!["param1", "param2"];
        assert!(!are_parameter_names_unique(names.into_iter().chain(vec!["param1", "param1"].into_iter())))
    }
}
