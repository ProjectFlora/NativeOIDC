use std::collections::BTreeSet;

/// Returns whether every parameter name appears only once.
///
/// Stops at the first duplicate name.
pub(crate) fn are_parameter_names_unique<'a>(mut names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    names.all(|name| seen.insert(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_pass() {
        assert!(are_parameter_names_unique(["a", "b", "c"].into_iter()));
    }

    #[test]
    fn a_duplicate_name_fails() {
        assert!(!are_parameter_names_unique(["a", "b", "a"].into_iter()));
    }

    #[test]
    fn an_empty_iterator_passes() {
        assert!(are_parameter_names_unique(std::iter::empty()));
    }
}
