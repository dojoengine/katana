use std::ops::{Deref, DerefMut, RangeBounds};

use roaring::RoaringTreemap;
use serde::{Deserialize, Serialize};

/// Stores a list of block numbers where a change occurred.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct BlockChangeList(IntegerSet);

impl BlockChangeList {
    pub fn new() -> Self {
        Self(IntegerSet::new())
    }

    /// Returns the block number of the last change that occurred **strictly before** `boundary`.
    ///
    /// Returns `None` if `boundary` is 0 or no changes exist before `boundary`.
    pub fn last_change_before(&self, boundary: u64) -> Option<u64> {
        boundary.checked_sub(1).and_then(|b| self.last_change_at_or_before(b))
    }

    /// Returns the block number of the most recent change **at or before** `block_number`.
    ///
    /// Returns `None` if the set is empty or no changes exist at or before `block_number`.
    pub fn last_change_at_or_before(&self, block_number: u64) -> Option<u64> {
        let rank = self.0.rank(block_number);
        if rank == 0 {
            return None;
        }

        self.0.select(rank - 1)
    }
}

impl Deref for BlockChangeList {
    type Target = IntegerSet;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for BlockChangeList {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const N: usize> From<[u64; N]> for BlockChangeList {
    fn from(arr: [u64; N]) -> Self {
        Self(IntegerSet::from(arr))
    }
}

impl<'a> IntoIterator for &'a BlockChangeList {
    type Item = u64;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

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

    /// Returns the maximum value in the set (if the set is non-empty).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use katana_db::models::list::IntegerSet;
    ///
    /// let mut is = IntegerSet::new();
    /// assert_eq!(is.max(), None);
    ///
    /// is.insert(3);
    /// is.insert(4);
    /// assert_eq!(is.max(), Some(4));
    /// ```
    pub fn max(&self) -> Option<u64> {
        self.0.max()
    }

    /// Returns the minimum value in the set (if the set is non-empty).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use katana_db::models::list::IntegerSet;
    ///
    /// let mut is = IntegerSet::new();
    /// assert_eq!(is.min(), None);
    ///
    /// is.insert(3);
    /// is.insert(4);
    /// assert_eq!(is.min(), Some(3));
    /// ```
    pub fn min(&self) -> Option<u64> {
        self.0.min()
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
    /// use katana_db::models::list::IntegerSet;
    ///
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
    /// use katana_db::models::list::IntegerSet;
    ///
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

#[cfg(test)]
mod tests {
    use super::BlockChangeList;

    #[test]
    fn empty_list_returns_none() {
        let list = BlockChangeList::new();
        assert_eq!(list.last_change_before(5), None);
    }

    #[test]
    fn boundary_zero_returns_none() {
        let list = BlockChangeList::from([0]);
        assert_eq!(list.last_change_before(0), None);
    }

    #[test]
    fn single_element_before_boundary() {
        let list = BlockChangeList::from([3]);
        assert_eq!(list.last_change_before(5), Some(3));
    }

    #[test]
    fn single_element_at_boundary() {
        let list = BlockChangeList::from([5]);
        assert_eq!(list.last_change_before(5), None);
    }

    #[test]
    fn single_element_after_boundary() {
        let list = BlockChangeList::from([10]);
        assert_eq!(list.last_change_before(5), None);
    }

    #[test]
    fn returns_closest_element_before_boundary() {
        let list = BlockChangeList::from([1, 3, 7, 10]);
        assert_eq!(list.last_change_before(5), Some(3));
    }

    #[test]
    fn element_immediately_before_boundary() {
        let list = BlockChangeList::from([4, 5]);
        assert_eq!(list.last_change_before(5), Some(4));
    }

    #[test]
    fn boundary_one_with_element_at_zero() {
        let list = BlockChangeList::from([0]);
        assert_eq!(list.last_change_before(1), Some(0));
    }

    #[test]
    fn boundary_one_no_element_at_zero() {
        let list = BlockChangeList::from([5]);
        assert_eq!(list.last_change_before(1), None);
    }

    #[test]
    fn all_elements_before_boundary() {
        let list = BlockChangeList::from([1, 2, 3]);
        assert_eq!(list.last_change_before(10), Some(3));
    }

    // --- last_change_at_or_before tests ---

    #[test]
    fn at_or_before_empty_list() {
        let list = BlockChangeList::new();
        assert_eq!(list.last_change_at_or_before(5), None);
    }

    #[test]
    fn at_or_before_exact_match() {
        let list = BlockChangeList::from([3, 5, 8]);
        assert_eq!(list.last_change_at_or_before(5), Some(5));
    }

    #[test]
    fn at_or_before_between_elements() {
        let list = BlockChangeList::from([3, 5, 8]);
        assert_eq!(list.last_change_at_or_before(6), Some(5));
    }

    #[test]
    fn at_or_before_less_than_all() {
        let list = BlockChangeList::from([5, 10]);
        assert_eq!(list.last_change_at_or_before(2), None);
    }

    #[test]
    fn at_or_before_greater_than_all() {
        let list = BlockChangeList::from([1, 3, 5]);
        assert_eq!(list.last_change_at_or_before(100), Some(5));
    }

    #[test]
    fn at_or_before_zero_with_element_at_zero() {
        let list = BlockChangeList::from([0, 5]);
        assert_eq!(list.last_change_at_or_before(0), Some(0));
    }

    #[test]
    fn at_or_before_zero_without_element_at_zero() {
        let list = BlockChangeList::from([1, 5]);
        assert_eq!(list.last_change_at_or_before(0), None);
    }

    #[test]
    fn at_or_before_last_element() {
        let list = BlockChangeList::from([1, 2, 5, 6, 10]);
        assert_eq!(list.last_change_at_or_before(10), Some(10));
    }

    #[test]
    fn at_or_before_first_element() {
        let list = BlockChangeList::from([1, 2, 5, 6, 10]);
        assert_eq!(list.last_change_at_or_before(1), Some(1));
    }
}
