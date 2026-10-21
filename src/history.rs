//! The browser-style navigation history of opened files.

use std::path::{Path, PathBuf};

/// A list of visited files with a current position, like a browser's back/forward history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    entries: Vec<PathBuf>,
    index: usize,
}

impl History {
    /// A history containing `initial` (if any) as its current entry.
    #[must_use]
    pub fn new(initial: Option<PathBuf>) -> Self {
        Self {
            entries: initial.into_iter().collect(),
            index: 0,
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Position of the current entry.
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Path> {
        self.entries.get(index).map(PathBuf::as_path)
    }

    #[must_use]
    pub fn current(&self) -> Option<&Path> {
        self.get(self.index)
    }

    /// The entry "back" would go to.
    #[must_use]
    pub fn previous(&self) -> Option<&Path> {
        self.index.checked_sub(1).and_then(|i| self.get(i))
    }

    /// The entry "forward" would go to.
    #[must_use]
    pub fn next(&self) -> Option<&Path> {
        self.get(self.index + 1)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        self.entries.iter().map(PathBuf::as_path)
    }

    #[must_use]
    pub const fn can_go_back(&self) -> bool {
        self.index > 0
    }

    #[must_use]
    pub const fn can_go_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    /// Makes `index` the current entry. Returns `false` (changing nothing) if it doesn't exist.
    pub fn set_index(&mut self, index: usize) -> bool {
        if index < self.entries.len() {
            self.index = index;
            true
        } else {
            false
        }
    }

    /// Records a newly visited file after the current entry, discarding any "forward" entries.
    pub fn push(&mut self, path: PathBuf) {
        self.entries.truncate(self.index + 1);
        self.entries.push(path);
        self.index = self.entries.len() - 1;
    }

    /// Removes the current entry and returns its former position, or `None` if empty.
    ///
    /// Afterwards the current index is out of step with what is displayed, so the caller is
    /// expected to follow up with [`History::set_index`] using one of
    /// [`History::close_candidates`].
    pub fn remove_current(&mut self) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let removed = self.index.min(self.entries.len() - 1);
        self.entries.remove(removed);
        self.index = removed.min(self.entries.len().saturating_sub(1));
        Some(removed)
    }

    /// After removing the entry at `removed`, the indices to try showing, best first: the
    /// nearest earlier entries, then the nearest later ones (which have shifted down into
    /// `removed..`).
    #[must_use]
    pub fn close_candidates(&self, removed: usize) -> Vec<usize> {
        (0..removed.min(self.entries.len()))
            .rev()
            .chain(removed..self.entries.len())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    fn history(names: &[&str], index: usize) -> History {
        let mut h = History::new(None);
        for n in names {
            h.push(p(n));
        }
        assert!(h.set_index(index));
        h
    }

    fn names(h: &History) -> Vec<String> {
        h.iter().map(|p| p.display().to_string()).collect()
    }

    #[test]
    fn new_with_and_without_a_file() {
        let h = History::new(None);
        assert!(h.is_empty());
        assert_eq!(h.current(), None);
        assert!(!h.can_go_back() && !h.can_go_forward());

        let h = History::new(Some(p("a")));
        assert_eq!(h.len(), 1);
        assert_eq!(h.current(), Some(Path::new("a")));
        assert!(!h.can_go_back() && !h.can_go_forward());
    }

    #[test]
    fn push_appends_and_moves_to_the_new_entry() {
        let mut h = History::new(Some(p("a")));
        h.push(p("b"));
        h.push(p("c"));
        assert_eq!(names(&h), ["a", "b", "c"]);
        assert_eq!(h.index(), 2);
        assert!(h.can_go_back());
        assert!(!h.can_go_forward());
    }

    #[test]
    fn push_into_an_empty_history() {
        let mut h = History::new(None);
        h.push(p("a"));
        assert_eq!(names(&h), ["a"]);
        assert_eq!(h.index(), 0);
    }

    #[test]
    fn push_discards_forward_entries() {
        let mut h = history(&["a", "b", "c"], 0);
        h.push(p("d"));
        assert_eq!(names(&h), ["a", "d"]);
        assert_eq!(h.index(), 1);
    }

    #[test]
    fn previous_and_next() {
        let h = history(&["a", "b", "c"], 1);
        assert_eq!(h.previous(), Some(Path::new("a")));
        assert_eq!(h.next(), Some(Path::new("c")));
        let h = history(&["a", "b", "c"], 0);
        assert_eq!(h.previous(), None);
        let h = history(&["a", "b", "c"], 2);
        assert_eq!(h.next(), None);
    }

    #[test]
    fn set_index_rejects_out_of_range() {
        let mut h = history(&["a", "b"], 0);
        assert!(!h.set_index(2));
        assert_eq!(h.index(), 0, "unchanged on failure");
        assert!(h.set_index(1));
        assert_eq!(h.index(), 1);
    }

    #[test]
    fn removing_the_only_entry_empties_the_history() {
        let mut h = History::new(Some(p("a")));
        assert_eq!(h.remove_current(), Some(0));
        assert!(h.is_empty());
        assert_eq!(h.index(), 0);
        assert!(h.close_candidates(0).is_empty());
        assert_eq!(h.remove_current(), None);
    }

    #[test]
    fn removing_the_last_entry_prefers_the_previous_one() {
        let mut h = history(&["a", "b", "c"], 2);
        let removed = h.remove_current().unwrap();
        assert_eq!(removed, 2);
        assert_eq!(names(&h), ["a", "b"]);
        assert_eq!(h.close_candidates(removed), [1, 0]);
    }

    #[test]
    fn removing_a_middle_entry_prefers_earlier_then_later() {
        let mut h = history(&["a", "b", "c", "d"], 1);
        let removed = h.remove_current().unwrap();
        assert_eq!(names(&h), ["a", "c", "d"]);
        // Earlier first (a), then what slid into the gap (c), then d.
        assert_eq!(h.close_candidates(removed), [0, 1, 2]);
    }

    #[test]
    fn removing_the_first_entry_falls_forward() {
        let mut h = history(&["a", "b", "c"], 0);
        let removed = h.remove_current().unwrap();
        assert_eq!(names(&h), ["b", "c"]);
        assert_eq!(h.close_candidates(removed), [0, 1]);
    }

    #[test]
    fn index_stays_valid_after_removal() {
        let mut h = history(&["a", "b", "c"], 2);
        h.remove_current();
        assert!(h.index() < h.len());
    }
}
