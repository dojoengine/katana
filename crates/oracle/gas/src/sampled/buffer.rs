use katana_primitives::block::GasPrices;

/// Gas prices buffer.
///
/// The buffer is implemented as a sliding window buffer i.e., once the buffer is full, the oldest
/// price is removed when a new gas price is inserted.
#[derive(Debug, Clone)]
pub struct GasPricesBuffer<const N: usize>(SlidingWindowBuffer<GasPrices, N>);

impl<const N: usize> GasPricesBuffer<N> {
    pub fn new() -> Self {
        Self(SlidingWindowBuffer::new())
    }

    pub fn push(&mut self, prices: GasPrices) {
        let _ = self.0.push(prices);
    }

    /// Calculate the average gas prices from the buffer.
    pub fn average(&self) -> GasPrices {
        if self.0.is_empty() {
            return GasPrices::MIN;
        }

        let sum = sum_gas_prices(self.0.iter());
        let eth_avg = sum.eth.get().div_ceil(self.0.len() as u128);
        let strk_avg = sum.strk.get().div_ceil(self.0.len() as u128);

        unsafe { GasPrices::new_unchecked(eth_avg, strk_avg) }
    }
}

/// Calculate the sum of gas prices from an iterator of GasPrices.
fn sum_gas_prices<'a, I: Iterator<Item = &'a GasPrices>>(iter: I) -> GasPrices {
    let (eth_sum, strk_sum) =
        iter.map(|p| (p.eth.get(), p.strk.get())).fold((0u128, 0u128), |acc, (eth, strk)| {
            (acc.0.saturating_add(eth), acc.1.saturating_add(strk))
        });

    // # SAFETY
    //
    // The minimum value for a GasPrice is 1 assuming it is created safely. So, the sum should at
    // minimum be 1u128. Otherwise, that's the responsibility of the caller to ensure the
    // unchecked values of GasPrices iterator are valid.
    unsafe { GasPrices::new_unchecked(eth_sum, strk_sum) }
}

/// A fixed-capacity circular buffer that implements a sliding window pattern.
///
/// This buffer maintains a fixed maximum capacity and automatically evicts the oldest
/// element when a new element is pushed to a full buffer (FIFO eviction policy). The buffer
/// uses a circular array internally for O(1) push and pop operations.
///
/// # Examples
///
/// ```
/// let mut buffer = SlidingWindowBuffer::<i32, 3>::new();
///
/// // Buffer grows until capacity
/// buffer.push(1); // [1]
/// buffer.push(2); // [1, 2]
/// buffer.push(3); // [1, 2, 3]
///
/// // At capacity, oldest element is evicted
/// let evicted = buffer.push(4); // [2, 3, 4]
/// assert_eq!(evicted, Some(1));
/// ```
#[derive(Debug, Clone)]
pub struct SlidingWindowBuffer<T, const N: usize> {
    /// The total number of elements currently in the buffer.
    len: usize,
    /// Index of the oldest element in the circular buffer.
    head: usize,
    /// The internal buffer where `None` represents an empty slot.
    buffer: [Option<T>; N],
}

impl<T: Clone, const N: usize> SlidingWindowBuffer<T, N> {
    /// Creates a new empty sliding window buffer.
    pub const fn new() -> Self {
        Self { buffer: [const { None }; N], head: 0, len: 0 }
    }

    /// Pushes a new element into the buffer.
    ///
    /// If the buffer is at capacity, the oldest element is evicted to make room
    /// for the new element. The evicted element is returned.
    ///
    /// # Arguments
    ///
    /// * `sample` - The element to add to the buffer
    ///
    /// # Returns
    ///
    /// * `Some(T)` - The evicted element if the buffer was at capacity
    /// * `None` - If the buffer had space and no element was evicted
    ///
    /// # Examples
    ///
    /// ```
    /// let mut buffer = SlidingWindowBuffer::<i32, 2>::new();
    /// assert_eq!(buffer.push(1), None); // Buffer not full
    /// assert_eq!(buffer.push(2), None); // Buffer now full
    /// assert_eq!(buffer.push(3), Some(1)); // Evicts oldest (1)
    /// ```
    pub fn push(&mut self, sample: T) -> Option<T> {
        let evicted = if self.len == N {
            let old_head = self.head;
            self.head = (self.head + 1) % N;
            self.buffer[old_head].take()
        } else {
            self.len += 1;
            None
        };

        let insert_pos = (self.head + self.len - 1) % N;
        self.buffer[insert_pos] = Some(sample);
        evicted
    }

    /// Removes and returns the oldest element from the buffer.
    ///
    /// This operation reduces the buffer's length by one. After popping,
    /// the next oldest element (if any) becomes the new head.
    ///
    /// # Returns
    ///
    /// * `Some(T)` - The oldest element if the buffer is not empty
    /// * `None` - If the buffer is empty
    ///
    /// # Examples
    ///
    /// ```
    /// let mut buffer = SlidingWindowBuffer::<i32, 3>::new();
    /// buffer.push(1);
    /// buffer.push(2);
    /// assert_eq!(buffer.pop(), Some(1)); // Removes oldest
    /// assert_eq!(buffer.pop(), Some(2));
    /// assert_eq!(buffer.pop(), None); // Buffer now empty
    /// ```
    #[allow(unused)]
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }

        let old_head = self.head;
        self.head = (self.head + 1) % N;
        self.len -= 1;
        self.buffer[old_head].take()
    }

    /// Returns the number of elements currently stored in the buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut buffer = SlidingWindowBuffer::<i32, 5>::new();
    /// assert_eq!(buffer.len(), 0);
    /// buffer.push(1);
    /// assert_eq!(buffer.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the buffer contains no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut buffer = SlidingWindowBuffer::<_, 3>::new();
    /// assert!(buffer.is_empty());
    /// buffer.push(1);
    /// assert!(!buffer.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns an iterator over the buffer's elements from oldest to newest.
    ///
    /// The iterator yields references to the elements in the order they were inserted,
    /// starting with the oldest element (at the head) and ending with the most recent.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut buffer = SlidingWindowBuffer::<i32, 4>::new();
    /// buffer.push(1);
    /// buffer.push(2);
    /// buffer.push(3);
    ///
    /// let values: Vec<i32> = buffer.iter().copied().collect();
    /// assert_eq!(values, vec![1, 2, 3]); // Oldest to newest
    /// ```
    pub fn iter(&self) -> Iter<'_, T, N> {
        Iter { buffer: self, index: 0 }
    }
}

/// An iterator over the elements in a [`SlidingWindowBuffer`].
///
/// This iterator yields references to elements in the buffer from oldest to newest.
/// It is created by the [`SlidingWindowBuffer::iter`] method.
pub struct Iter<'a, T, const N: usize> {
    buffer: &'a SlidingWindowBuffer<T, N>,
    index: usize,
}

impl<'a, T, const N: usize> Iterator for Iter<'a, T, N> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.buffer.len {
            return None;
        }

        let idx = (self.buffer.head + self.index) % N;
        self.index += 1;
        self.buffer.buffer[idx].as_ref()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.buffer.len.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl<T, const N: usize> ExactSizeIterator for Iter<'_, T, N> {
    fn len(&self) -> usize {
        self.buffer.len.saturating_sub(self.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_size_limit() {
        const BUFFER_SIZE: usize = 5;
        let mut buffer = SlidingWindowBuffer::<u128, BUFFER_SIZE>::new();

        // Fill up buffer
        for value in 0..BUFFER_SIZE {
            buffer.push(value as u128);
        }

        // Check if buffer size is maintained
        assert_eq!(buffer.len(), BUFFER_SIZE);

        // Fill up buffer
        for (expected_value, elem) in buffer.iter().enumerate() {
            assert_eq!(expected_value as u128, *elem)
        }

        // check first in first out
        for i in BUFFER_SIZE..(BUFFER_SIZE * 2) {
            let removed = buffer.push(i as u128);
            assert_eq!(removed, Some((i - BUFFER_SIZE) as _));
        }
    }

    #[test]
    fn gas_prices_buffer_average_empty() {
        let buffer = GasPricesBuffer::<5>::new();
        let average = buffer.average();
        assert_eq!(average, GasPrices::MIN);
    }

    #[test]
    fn gas_prices_buffer_average_single_element() {
        let mut buffer = GasPricesBuffer::<5>::new();

        let gas_price = unsafe { GasPrices::new_unchecked(100, 200) };
        buffer.push(gas_price);

        let average = buffer.average();
        assert_eq!(average.eth.get(), 100);
        assert_eq!(average.strk.get(), 200);
    }

    #[test]
    fn gas_prices_buffer_average_multiple_elements() {
        let mut buffer = GasPricesBuffer::<5>::new();

        // Add test gas prices
        let prices = [
            unsafe { GasPrices::new_unchecked(100, 150) },
            unsafe { GasPrices::new_unchecked(200, 250) },
            unsafe { GasPrices::new_unchecked(300, 350) },
        ];

        for price in prices {
            buffer.push(price);
        }

        let average = buffer.average();
        // Expected: eth = (100 + 200 + 300) / 3 = 200, strk = (150 + 250 + 350) / 3 = 250
        assert_eq!(average.eth.get(), 200);
        assert_eq!(average.strk.get(), 250);
    }

    #[test]
    fn gas_prices_buffer_average_ceiling_division() {
        let mut buffer = GasPricesBuffer::<5>::new();

        // Add prices that don't divide evenly
        let prices =
            unsafe { [GasPrices::new_unchecked(10, 11), GasPrices::new_unchecked(20, 22)] };

        for price in prices {
            buffer.push(price);
        }

        let average = buffer.average();
        // Expected: eth = (10 + 20) / 2 = 15, strk = (11 + 22) / 2 = 16.5 -> ceil to 17
        assert_eq!(average.eth.get(), 15);
        assert_eq!(average.strk.get(), 17); // Ceiling division
    }

    #[test]
    fn gas_prices_buffer_average_large_numbers() {
        let mut buffer = GasPricesBuffer::<5>::new();

        let max_val = u128::MAX / 2; // Use half of max to avoid overflow
        let prices = unsafe { [GasPrices::new_unchecked(max_val, max_val), GasPrices::MIN] };

        for price in prices {
            buffer.push(price);
        }

        let average = buffer.average();
        // Test that large numbers are handled correctly
        let expected_eth = (max_val + 1).div_ceil(2);
        let expected_strk = (max_val + 1).div_ceil(2);
        assert_eq!(average.eth.get(), expected_eth);
        assert_eq!(average.strk.get(), expected_strk);
    }

    #[test]
    fn sliding_window_iterator_empty_buffer() {
        let buffer = SlidingWindowBuffer::<i32, 5>::new();
        let mut iter = buffer.iter();

        assert_eq!(iter.next(), None);
        assert_eq!(iter.len(), 0);
        assert_eq!(iter.size_hint(), (0, Some(0)));

        let collected: Vec<_> = buffer.iter().collect();
        assert!(collected.is_empty());
    }

    #[test]
    fn sliding_window_iterator_partial_buffer() {
        let mut buffer = SlidingWindowBuffer::<i32, 5>::new();
        buffer.push(1);
        buffer.push(2);
        buffer.push(3);

        let collected: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(collected, vec![1, 2, 3]);

        let mut iter = buffer.iter();
        assert_eq!(iter.len(), 3);
        assert_eq!(iter.size_hint(), (3, Some(3)));

        assert_eq!(iter.next(), Some(&1));
        assert_eq!(iter.len(), 2);
        assert_eq!(iter.size_hint(), (2, Some(2)));

        assert_eq!(iter.next(), Some(&2));
        assert_eq!(iter.len(), 1);
        assert_eq!(iter.size_hint(), (1, Some(1)));

        assert_eq!(iter.next(), Some(&3));
        assert_eq!(iter.len(), 0);
        assert_eq!(iter.size_hint(), (0, Some(0)));

        assert_eq!(iter.next(), None);
    }

    #[test]
    fn sliding_window_iterator_full_buffer_no_wrap() {
        let mut buffer = SlidingWindowBuffer::<i32, 5>::new();
        for i in 1..=5 {
            buffer.push(i);
        }

        let collected: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(collected, vec![1, 2, 3, 4, 5]);

        assert_eq!(buffer.iter().len(), 5);
        assert_eq!(buffer.iter().count(), 5);
    }

    #[test]
    fn sliding_window_iterator_with_wraparound() {
        let mut buffer = SlidingWindowBuffer::<i32, 3>::new();

        // Fill the buffer
        buffer.push(1);
        buffer.push(2);
        buffer.push(3);

        // Push more elements to cause wraparound
        buffer.push(4); // evicts 1
        buffer.push(5); // evicts 2

        // Buffer should now contain [3, 4, 5] with head pointing to 3
        let collected: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(collected, vec![3, 4, 5]);

        // Push one more to verify wraparound works correctly
        buffer.push(6); // evicts 3
        let collected: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(collected, vec![4, 5, 6]);
    }

    #[test]
    fn sliding_window_iterator_complex_wraparound() {
        let mut buffer = SlidingWindowBuffer::<i32, 4>::new();

        // Create a complex wraparound scenario
        for i in 1..=10 {
            buffer.push(i);
        }

        // Buffer should only contain [7, 8, 9, 10] due to fixed size of 4.
        let collected: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(collected, vec![7, 8, 9, 10]);

        // Test iterator correctness
        let mut iter = buffer.iter();
        assert_eq!(iter.next(), Some(&7));
        assert_eq!(iter.next(), Some(&8));
        assert_eq!(iter.next(), Some(&9));
        assert_eq!(iter.next(), Some(&10));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn sliding_window_iterator_after_pop() {
        let mut buffer = SlidingWindowBuffer::<i32, 5>::new();

        // Fill buffer
        for i in 1..=5 {
            buffer.push(i);
        }

        // Pop some elements
        assert_eq!(buffer.pop(), Some(1));
        assert_eq!(buffer.pop(), Some(2));

        // Iterator should show remaining elements
        let collected: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(collected, vec![3, 4, 5]);
        assert_eq!(buffer.iter().len(), 3);

        // Add more elements
        buffer.push(6);
        buffer.push(7);

        let collected: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(collected, vec![3, 4, 5, 6, 7]);
    }

    #[test]
    fn sliding_window_iterator_multiple_iterations() {
        let mut buffer = SlidingWindowBuffer::<i32, 3>::new();
        buffer.push(1);
        buffer.push(2);
        buffer.push(3);

        // Multiple iterations should work independently
        let first: Vec<_> = buffer.iter().copied().collect();
        let second: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(first, second);
        assert_eq!(first, vec![1, 2, 3]);
    }

    #[test]
    fn sliding_window_iterator_size_hint_accuracy() {
        let mut buffer = SlidingWindowBuffer::<i32, 4>::new();

        // Test size hint at different stages
        let iter = buffer.iter();
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.len(), 0);

        buffer.push(1);
        let mut iter = buffer.iter();
        assert_eq!(iter.size_hint(), (1, Some(1)));
        assert_eq!(iter.len(), 1);

        // Consume one element
        iter.next();
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.len(), 0);

        // Fill buffer
        buffer.push(2);
        buffer.push(3);
        buffer.push(4);

        let mut iter = buffer.iter();
        assert_eq!(iter.size_hint(), (4, Some(4)));

        // Partially consume
        iter.next();
        iter.next();
        assert_eq!(iter.size_hint(), (2, Some(2)));
        assert_eq!(iter.len(), 2);
    }

    #[test]
    fn sliding_window_iterator_edge_case_single_element() {
        let mut buffer = SlidingWindowBuffer::<i32, 1>::new();

        buffer.push(1);
        assert_eq!(buffer.iter().copied().collect::<Vec<_>>(), vec![1]);

        buffer.push(2); // evicts 1
        assert_eq!(buffer.iter().copied().collect::<Vec<_>>(), vec![2]);
    }
}
