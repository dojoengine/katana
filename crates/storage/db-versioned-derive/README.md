# Katana DB Versioned Derive

A procedural macro for automatically generating versioned database types to maintain backward compatibility.

## Problem

When primitive types in `katana-primitives` change, it can break database format compatibility. Previously, adding a new version required:
- Manually copying entire struct definitions
- Writing `From` implementations for each struct
- Handling field conversions by hand
- Maintaining enum variants for types with multiple versions

This resulted in hundreds of lines of boilerplate code for each version.

## Solution

The `#[derive(Versioned)]` macro automatically generates:
- Version-specific struct/enum definitions
- `From` trait implementations for conversions
- Proper serde derives for serialization
- Support for version-specific field types

## Usage

### Basic Example

```rust
use katana_db_versioned_derive::Versioned;

#[derive(Versioned)]
#[versioned(current = "katana_primitives::transaction")]
pub struct InvokeTxV3 {
    pub chain_id: ChainId,
    pub sender_address: ContractAddress,
    pub nonce: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    
    // Field with version-specific types
    #[versioned(
        v6 = "v6::ResourceBoundsMapping",
        v7 = "v7::ResourceBoundsMapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
}
```

### Version-Specific Types

Define version-specific types that differ from current primitives:

```rust
pub mod v6 {
    use super::*;
    
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ResourceBoundsMapping {
        pub l1_gas: ResourceBounds,
        pub l2_gas: ResourceBounds,
    }
    
    // User provides conversion logic
    impl From<ResourceBoundsMapping> for fee::ResourceBoundsMapping {
        fn from(v6: ResourceBoundsMapping) -> Self {
            // Custom conversion logic
            fee::ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: v6.l1_gas,
                l2_gas: v6.l2_gas,
            })
        }
    }
}
```

### Generated Code

The macro generates:

1. **Current struct** with proper derives:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct InvokeTxV3 { /* fields */ }
```

2. **Version-specific modules**:
```rust
pub mod v6 {
    pub struct InvokeTxV3 {
        // Uses v6::ResourceBoundsMapping for resource_bounds
        pub resource_bounds: v6::ResourceBoundsMapping,
        // ... other fields
    }
    
    impl From<InvokeTxV3> for katana_primitives::transaction::InvokeTxV3 {
        fn from(v6: InvokeTxV3) -> Self {
            Self {
                // Uses .into() for each field
                resource_bounds: v6.resource_bounds.into(),
                // ...
            }
        }
    }
}
```

### Enum Support

The macro also supports enums:

```rust
#[derive(Versioned)]
#[versioned(current = "katana_primitives::transaction")]
pub enum InvokeTx {
    V0(transaction::InvokeTxV0),
    V1(transaction::InvokeTxV1),
    V3(InvokeTxV3),
}
```

### Field Attributes

- `#[versioned(v6 = "path::to::Type")]` - Specify version-specific type
- `#[versioned(added_in = "v7")]` - Field added in this version
- `#[versioned(removed_after = "v7")]` - Field removed after this version

## Adding a New Version

When adding version 8:

1. Define any version-specific types:
```rust
pub mod v8 {
    pub struct NewResourceBounds {
        // v8 specific fields
    }
    
    impl From<NewResourceBounds> for fee::ResourceBoundsMapping {
        fn from(v8: NewResourceBounds) -> Self {
            // Conversion logic
        }
    }
}
```

2. Update the field attribute:
```rust
#[versioned(
    v6 = "v6::ResourceBoundsMapping",
    v7 = "v7::ResourceBoundsMapping",
    v8 = "v8::NewResourceBounds"  // Add v8
)]
pub resource_bounds: ResourceBoundsMapping,
```

That's it! The macro handles all the boilerplate.

## Benefits

- **80% less boilerplate code** when adding versions
- **Type-safe** conversions with compile-time verification
- **Self-documenting** version history in attributes
- **User control** over conversion logic via `From` traits
- **Automatic** generation of all repetitive code

## How It Works

1. **Parse** - The macro parses struct/enum definitions and versioned attributes
2. **Generate modules** - Creates version-specific modules based on attributes
3. **Create structs** - Generates structs with version-specific field types
4. **Implement From** - Creates From implementations calling `.into()` on each field
5. **User provides** - Custom From implementations for version-specific types

The macro assumes each field can be converted via `.into()`, giving users full control over the conversion logic by implementing `From` traits for their custom types.