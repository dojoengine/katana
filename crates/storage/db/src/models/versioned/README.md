# Database Versioning System

This module provides a macro-based versioning system for database types to ensure backward compatibility when the primitive types change.

## Problem

When primitive types in `katana-primitives` change, it can break the database format, making databases created with previous Katana versions incompatible. The versioning system ensures that:

1. The database is aware of format changes
2. Old data can still be deserialized correctly
3. New versions can be added with minimal boilerplate

## Solution: Macro-Based Versioning

The `versioned_type!` macro automatically generates versioned enums with all necessary trait implementations.

## Usage

### Basic Setup

To create a versioned type, use the `versioned_type!` macro:

```rust
use crate::versioned_type;

versioned_type! {
    VersionedTx {
        V6 => v6::Tx,
        V7 => katana_primitives::transaction::Tx,
    }
}
```

This automatically generates:
- The versioned enum with all variants
- `From` trait implementations for conversions
- `Compress` and `Decompress` implementations with fallback chain
- Conversion to/from the latest version

### Adding a New Version

When the primitive types change in a breaking way:

1. **Update the database version** in `crates/storage/db/src/version.rs`:
   ```rust
   pub const CURRENT_DB_VERSION: Version = Version::new(8); // Increment version
   ```

2. **Create a new version module** (e.g., `v7.rs`) that contains only the types that changed:
   ```rust
   // crates/storage/db/src/models/versioned/transaction/v7.rs
   use serde::{Deserialize, Serialize};
   
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct NewFieldType {
       pub new_field: u64,
       // ... other fields
   }
   
   // Implement conversion to the current primitive type
   impl From<NewFieldType> for katana_primitives::NewFieldType {
       fn from(v7: NewFieldType) -> Self {
           // Handle conversion
       }
   }
   ```

3. **Update the versioned type declaration**:
   ```rust
   versioned_type! {
       VersionedTx {
           V6 => v6::Tx,
           V7 => v7::Tx,
           V8 => katana_primitives::transaction::Tx,  // Latest version
       }
   }
   ```

That's it! The macro handles all the boilerplate.

## How It Works

### Serialization
- New data is always serialized using the latest version variant
- The versioned enum wrapper ensures version information is preserved

### Deserialization
The `Decompress` implementation tries deserialization in this order:
1. First, as the versioned enum itself (for data that was already versioned)
2. Then, as the latest version type (for recent unversioned data)
3. Finally, falling back through older versions in reverse order

This ensures maximum compatibility with both old and new data formats.

### Conversions
- `From<LatestType>` creates a versioned enum with the latest variant
- `From<VersionedType>` converts any version to the latest type
- Each old version module must implement conversion to the current types

## Best Practices

1. **Only define changed types**: In version modules, only include types that actually changed
2. **Preserve field order**: When possible, maintain the same field order for serialization compatibility
3. **Document changes**: Add comments explaining what changed in each version
4. **Test thoroughly**: Add tests for round-trip serialization and cross-version compatibility

## Example: Complete Version Addition

Here's a complete example of adding V8 when a field type changes:

```rust
// 1. Create v7.rs with the old type definition
// crates/storage/db/src/models/versioned/transaction/v7.rs
mod v7 {
    use serde::{Deserialize, Serialize};
    
    // This is how ResourceBounds looked in V7
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ResourceBounds {
        pub max_amount: u64,
        pub max_price: u64,
    }
    
    // Conversion to the new format
    impl From<ResourceBounds> for katana_primitives::ResourceBounds {
        fn from(v7: ResourceBounds) -> Self {
            Self {
                max_amount: v7.max_amount,
                max_price_per_unit: v7.max_price, // Field renamed
            }
        }
    }
}

// 2. Update the versioned type
versioned_type! {
    VersionedTx {
        V6 => v6::Tx,
        V7 => v7::Tx,  // Now points to our v7 module
        V8 => katana_primitives::transaction::Tx,  // Latest
    }
}

// 3. Update CURRENT_DB_VERSION to 8
```

## Testing

Always test version compatibility:

```rust
#[test]
fn test_v7_to_v8_migration() {
    // Create V7 data
    let v7_tx = v7::Tx { /* ... */ };
    let versioned = VersionedTx::V7(v7_tx);
    
    // Convert to latest
    let v8_tx: Tx = versioned.into();
    
    // Verify conversion worked correctly
    assert_eq!(v8_tx.field, expected_value);
}
```

## Benefits

This macro-based approach reduces version addition from hundreds of lines to just:
1. One line in the versioned type declaration
2. A module with only the types that changed
3. Conversion implementations for those types

The rest is handled automatically!