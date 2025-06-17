use std::ops::RangeBounds;

use roaring::RoaringTreemap;
use serde::{Deserialize, Serialize};

/// Stores a list of block numbers.
/// Mainly used for changeset tables to store the list of block numbers where a change occurred.
pub type BlockList = IntegerSet;

/// A set for storing integer values.
///
/// The list is stored in a Roaring bitmap data structure as it uses less space compared to a normal
/// bitmap or even a naive array with similar cardinality.
///
/// See <https://www.roaringbitmap.org/>.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct IntegerSet(RoaringTreemap);

impl IntegerSet {
    pub fn new() -> Self {
        Self(RoaringTreemap::new())
    }

    /// Insert a new number to the set.
    pub fn insert(&mut self, num: u64) {
        self.0.insert(num);
    }

    /// Removes a value from the set. Returns `true` if the value was present in the set.
    pub fn remove(&mut self, num: u64) -> bool {
        self.0.remove(num)
    }

    /// Checks if the set contains the given number.
    pub fn contains(&self, num: u64) -> bool {
        self.0.contains(num)
    }

    /// Returns the number of elements in the set that are smaller or equal to the given `value`.
    pub fn rank(&self, value: u64) -> u64 {
        self.0.rank(value)
    }

    /// Returns the `n`th integer in the set or `None` if `n >= len()`.
    pub fn select(&self, n: u64) -> Option<u64> {
        self.0.select(n)
    }

    /// Removes a range of values.
    ///
    /// # Returns
    ///
    /// Returns the number of removed values.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut is = IntegerSet::new();
    /// is.insert(2);
    /// is.insert(3);
    /// assert_eq!(is.remove_range(2..4), 2);
    /// ```
    pub fn remove_range<R: RangeBounds<u64>>(&mut self, range: R) -> u64 {
        self.0.remove_range(range)
    }

    /// Iterator over each value stored in the [`IntegerSet`], guarantees values are ordered by
    /// value.
    pub fn iter(&self) -> Iter<'_> {
        Iter { inner: self.0.iter() }
    }

    /// Returns the number of distinct integers added to the set.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut is = IntegerSet::new();
    /// assert_eq!(is.len(), 0);
    ///
    /// is.insert(3);
    /// assert_eq!(is.len(), 1);
    ///
    /// is.insert(3);
    /// is.insert(4);
    /// assert_eq!(is.len(), 2);
    /// ```
    pub fn len(&self) -> u64 {
        self.0.len()
    }

    /// Returns `true` if there are no integers in this set.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<const N: usize> From<[u64; N]> for IntegerSet {
    fn from(arr: [u64; N]) -> Self {
        Self(RoaringTreemap::from_iter(arr))
    }
}

impl<'a> IntoIterator for &'a IntegerSet {
    type Item = u64;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// An iterator for `RoaringTreemap`.
#[allow(missing_debug_implementations)]
pub struct Iter<'a> {
    inner: roaring::treemap::Iter<'a>,
}

impl Iterator for Iter<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }

    #[inline]
    fn fold<B, F>(self, init: B, f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        self.inner.fold(init, f)
    }
}
