use std::collections::{vec_deque, VecDeque};

use katana_primitives::block::GasPrices;

#[derive(Debug, Clone)]
pub struct GasPricesBuffer(SlidingWindowBuffer<GasPrices>);

impl GasPricesBuffer {
    pub fn new(size: usize) -> Self {
        Self(SlidingWindowBuffer::new(size))
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

#[derive(Debug, Clone)]
pub struct SlidingWindowBuffer<T>(VecDeque<T>);

impl<T> SlidingWindowBuffer<T> {
    /// Creates a new buffer with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        Self(VecDeque::with_capacity(capacity))
    }

    /// Pushes a new element into the buffer, evicting the oldest element if the buffer is full.
    ///
    /// Returns the evicted element, if any.
    pub fn push(&mut self, sample: T) -> Option<T> {
        let evicted = if self.len() == self.capacity() { self.pop() } else { None };
        self.0.push_back(sample);
        evicted
    }

    /// Removes the oldest element from the buffer.
    pub fn pop(&mut self) -> Option<T> {
        self.0.pop_front()
    }

    /// Returns the total number of elements in the buffer.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the buffer is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut buffer = SlidingWindowBuffer::new(3);
    /// assert!(buffer.is_empty());
    /// buffer.push(1);
    /// assert!(!buffer.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the total number of elements can be stored in the buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// let buf: SlidingWindowBuffer<i32> = SlidingWindowBuffer::new(10);
    /// assert!(buf.capacity() >= 10);
    /// ```
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    /// Returns an iterator over the elements in the buffer from the oldest to the newest.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter(self.0.iter())
    }
}

#[derive(Debug, Clone)]
pub struct Iter<'a, T>(vec_deque::Iter<'a, T>);

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_size_limit() {
        const BUFFER_SIZE: usize = 5;
        let mut buffer = SlidingWindowBuffer::new(BUFFER_SIZE);

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
        let buffer = GasPricesBuffer::new(5);
        let average = buffer.average();
        assert_eq!(average, GasPrices::MIN);
    }

    #[test]
    fn gas_prices_buffer_average_single_element() {
        let mut buffer = GasPricesBuffer::new(5);

        let gas_price = unsafe { GasPrices::new_unchecked(100, 200) };
        buffer.push(gas_price);

        let average = buffer.average();
        assert_eq!(average.eth.get(), 100);
        assert_eq!(average.strk.get(), 200);
    }

    #[test]
    fn gas_prices_buffer_average_multiple_elements() {
        let mut buffer = GasPricesBuffer::new(5);

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
        let mut buffer = GasPricesBuffer::new(5);

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
        let mut buffer = GasPricesBuffer::new(5);

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
}
