//! The font families this machine has, for the two settings that choose one.
//!
//! A list of names and nothing else: the interface draws with them, so what it needs is what to put
//! in a `font-family`, and a name it cannot resolve falls back to the next one in the stack. Which
//! is also why a machine whose fonts cannot be read is not an error — the two settings still offer
//! the families that ship with the application, and somebody who wants another one can type it.
//!
//! Asked of the platform rather than read out of the font files, and that is the point of a system
//! source: the names come back spelled the way this machine spells them. On an installation in
//! Chinese that means 微软雅黑, 宋体 and 楷体 — the names somebody sitting at that computer sees in
//! every other application, and the ones they will search for. A name table carries both spellings,
//! and choosing between them here would be this build deciding what language the machine is in.

use std::collections::BTreeSet;

/// The families installed on this machine, sorted, as this system spells them.
///
/// Sorted because a list of two hundred names in whatever order the platform returned them is a list
/// nobody can find anything in, and deduplicated because a family with four faces (regular, bold,
/// italic, bold-italic) is one family.
pub fn list_families() -> Vec<String> {
    let source = font_kit::source::SystemSource::new();
    match source.all_families() {
        Ok(families) => families
            .into_iter()
            .map(|family| family.trim().to_owned())
            .filter(|family| !family.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_families_are_sorted_and_unique() {
        let families = list_families();
        // A machine with no fonts at all is not this test's business — the list is allowed to be
        // empty, and what is asserted is the shape of it when it is not.
        let mut sorted = families.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(families, sorted, "the list is sorted and has no family twice");

        assert!(
            families
                .iter()
                .all(|family| family.trim() == family && !family.is_empty()),
            "a name with padding or nothing in it would be a line that does nothing when clicked"
        );
    }
}
