pub use arbitrary::{Arbitrary, Unstructured};
pub use katana_utils_macro::mock_provider;

#[cfg(feature = "node")]
pub mod node;
mod signal;
mod tx_waiter;

#[cfg(feature = "node")]
pub use node::TestNode;
pub use signal::wait_shutdown_signals;
pub use tx_waiter::*;

/// Generate a random bytes vector of the given size.
pub fn random_bytes(size: usize) -> Vec<u8> {
    (0..size).map(|_| rand::random::<u8>()).collect()
}

/// Generate a random instance of the given type using the [`Arbitrary`](arbitrary::Arbitrary)
/// trait.
///
/// # Examples
///
/// ```
/// # use arbitrary::Arbitrary;
/// # #[derive(Arbitrary)]
/// # struct MyStruct {
/// #     value: u32,
/// # }
/// // Generate a random instance with automatically generated data
/// let my_struct: MyStruct = arbitrary!(MyStruct);
///
/// // Generate a random instance with provided Unstructured data
/// let data = vec![1, 2, 3, 4, 5];
/// let mut unstructured = arbitrary::Unstructured::new(&data);
/// let my_struct: MyStruct = arbitrary!(MyStruct, unstructured);
/// ```
#[macro_export]
macro_rules! arbitrary {
    ($type:ty) => {{
        let data = $crate::random_bytes(<$type as $crate::Arbitrary>::size_hint(0).0);
        let mut data = $crate::Unstructured::new(&data);
        <$type as $crate::Arbitrary>::arbitrary(&mut data)
            .expect(&format!("failed to generate arbitrary {}", std::any::type_name::<$type>()))
    }};
    ($type:ty, $data:expr) => {{
        <$type as $crate::Arbitrary>::arbitrary(&mut $data)
            .expect(&format!("failed to generate arbitrary {}", std::any::type_name::<$type>()))
    }};
}
