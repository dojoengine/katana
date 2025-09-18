//! Clean example demonstrating the Versioned attribute macro
//!
//! This shows the correct way to use the macro without generating duplicate structs.

use katana_db_versioned_derive::versioned;
use serde::{Deserialize, Serialize};

// Define version-specific types that differ between versions
pub mod types_v6 {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
    pub struct OldBounds {
        pub max_amount: u64,
        pub price: u128,
    }
}

pub mod types_v7 {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
    pub struct NewBounds {
        pub max_amount: u64,
        pub price_per_unit: u128, // Field renamed
        pub priority: u8,         // Field added
    }
}

// Current version of the bounds type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentBounds {
    pub max_amount: u64,
    pub price_per_unit: u128,
    pub priority: u8,
    pub discount: u32, // Added in current version (using u32 instead of f32 for Eq trait)
}

// Implement conversions from old versions to current
impl From<types_v6::OldBounds> for CurrentBounds {
    fn from(v6: types_v6::OldBounds) -> Self {
        Self {
            max_amount: v6.max_amount,
            price_per_unit: v6.price, // Map old field name
            priority: 0,              // Default for missing field
            discount: 0,              // Default for missing field
        }
    }
}

impl From<types_v7::NewBounds> for CurrentBounds {
    fn from(v7: types_v7::NewBounds) -> Self {
        Self {
            max_amount: v7.max_amount,
            price_per_unit: v7.price_per_unit,
            priority: v7.priority,
            discount: 0, // Default for field added after v7
        }
    }
}

#[versioned(version = 8)]
mod versioned {
    #[versioned]
    pub struct MyTransaction {
        pub id: u64,
        pub sender: String,
        #[version(6 = "types_v6::OldBounds", 7 = "types_v7::NewBounds")]
        pub bounds: CurrentBounds,
    }

    #[versioned]
    pub struct OtherStruct {
        pub id: u64,
        #[version(v6 = "types_v6::OldName")]
        pub name: String,
    }
}

pub mod versioned {
    use crate::models::versioned::example_clean::CurrentBounds;

    versioned_type! {
        VersionedTx {
            V6 => v6::MyTransaction,
            V7 => v7::MyTransaction,
            V8 => MyTransaction,
        }
    }

    pub struct MyTransaction {
        pub id: u64,
        pub sender: String,
        pub bounds: CurrentBounds,
    }

    pub mod v6 {
        use super::super::types_v6::OldBounds;
        use super::super::types_v6::OldName;

        pub struct MyTransaction {
            pub id: u64,
            pub sender: String,
            pub bounds: OldBounds,
        }

        pub struct OtherStruct {
            pub id: u64,
            pub name: OldName,
        }
    }

    pub mod v7 {
        use super::super::types_v7::NewBounds;

        pub struct MyTransaction {
            pub id: u64,
            pub sender: String,
            pub bounds: NewBounds,
        }
    }
}

// Now the actual struct with the macro
// The macro includes this struct in its output along with the version modules
#[versioned]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyTransaction {
    pub id: u64,
    pub sender: String,

    // This field has different types in different versions
    #[version(v6 = "types_v6::OldBounds", v7 = "types_v7::NewBounds")]
    pub bounds: CurrentBounds,

    pub data: Vec<u8>,
}

// The macro generates these modules:
//
// pub mod v6 {
//     use super::*;
//
//     #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
//     pub struct MyTransaction {
//         pub id: u64,
//         pub sender: String,
//         pub bounds: types_v6::OldBounds,  // Uses v6 type
//         pub data: Vec<u8>,
//     }
//
//     impl From<MyTransaction> for super::MyTransaction {
//         fn from(v6: MyTransaction) -> Self {
//             Self {
//                 id: v6.id.into(),
//                 sender: v6.sender.into(),
//                 bounds: v6.bounds.into(),  // Uses the From impl we defined
//                 data: v6.data.into(),
//             }
//         }
//     }
// }
//
// pub mod v7 {
//     // Similar structure with types_v7::NewBounds
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versioned_macro() {
        // Create current version
        let tx = MyTransaction {
            id: 1,
            sender: "Alice".to_string(),
            bounds: CurrentBounds {
                max_amount: 1000,
                price_per_unit: 100,
                priority: 5,
                discount: 10,
            },
            data: vec![1, 2, 3],
        };

        assert_eq!(tx.id, 1);
        assert_eq!(tx.bounds.discount, 10);
    }

    #[test]
    fn test_v6_conversion() {
        // Create a v6 transaction
        let v6_tx = v6::MyTransaction {
            id: 42,
            sender: "Bob".to_string(),
            bounds: types_v6::OldBounds { max_amount: 500, price: 50 },
            data: vec![4, 5, 6],
        };

        // Convert to current version
        let current: MyTransaction = v6_tx.into();

        assert_eq!(current.id, 42);
        assert_eq!(current.sender, "Bob");
        assert_eq!(current.bounds.max_amount, 500);
        assert_eq!(current.bounds.price_per_unit, 50);
        assert_eq!(current.bounds.priority, 0); // Default value
        assert_eq!(current.bounds.discount, 0); // Default value
    }
}
