pub mod node;
mod tx_waiter;

pub use node::TestNode;
pub use tx_waiter::*;

#[inline(always)]
pub fn random_bytes(size: usize) -> Vec<u8> {
    (0..size).map(|_| rand::random::<u8>()).collect()
}
