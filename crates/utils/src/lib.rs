pub mod node;
mod tx_waiter;

pub use node::TestNode;
pub use tx_waiter::*;

/// Generate a random bytes vector of the given size.
pub fn random_bytes(size: usize) -> Vec<u8> {
    (0..size).map(|_| rand::random::<u8>()).collect()
}

/// Generate a random instance of the given type.
#[macro_export]
macro_rules! arbitrary {
    ($type:ty) => {{
        let bytes = $crate::random_bytes(<$type as arbitrary::Arbitrary>::size_hint(0).0);
        let mut data = arbitrary::Unstructured::new(&bytes);
        <$type as arbitrary::Arbitrary>::arbitrary(&mut data)
            .expect(&format!("failed to generate arbitrary {}", std::any::type_name::<$type>()))
    }};
}
