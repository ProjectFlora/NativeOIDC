//! Checks whether a list of parameter names are all unique.
//! Used to reject authorization requests that have duplicate parameter names.

use std::collections::BTreeSet;

/// Return `true` if every name appears exactly once, `false` otherwise.
pub(crate) fn are_parameter_names_unique<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen: BTreeSet<&'a str> = BTreeSet::new();
    names.all(|name| seen.insert(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_unique() {
        assert!(are_parameter_names_unique(["a", "b", "c"].iter()));
    }

    #[test]
    fn first_duplicate_fails() {
        assert!(!are_parameter_names_unique(["a", "a", "b"].iter()));
    }

    #[test]
    fn multiple_duplicates_fail() {
        assert!(!are_parameter_names_unique(["a", "b", "a", "b"].iter()));
    }

    #[test]
    fn empty_iterator_passes() {
        assert!(are_parameter_names_unique([].iter()));
    }
}
