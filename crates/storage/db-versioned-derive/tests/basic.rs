use katana_db_versioned_derive::Versioned;
use serde::{Deserialize, Serialize};

// Test basic struct versioning
#[test]
fn test_basic_struct() {
    #[derive(Versioned)]
    #[versioned(current = "example")]
    pub struct MyStruct {
        pub field1: String,
        pub field2: u32,
    }
    
    // This should compile and generate proper derives
    let s = MyStruct {
        field1: "test".to_string(),
        field2: 42,
    };
    
    assert_eq!(s.field1, "test");
    assert_eq!(s.field2, 42);
}

// Test struct with versioned fields
#[test]
fn test_versioned_fields() {
    // Mock types for testing
    pub mod v6 {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        pub struct CustomType {
            pub value: u32,
        }
        
        impl From<CustomType> for super::CurrentCustomType {
            fn from(v6: CustomType) -> Self {
                super::CurrentCustomType {
                    value: v6.value as u64,
                }
            }
        }
    }
    
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CurrentCustomType {
        pub value: u64,
    }
    
    impl From<CurrentCustomType> for CurrentCustomType {
        fn from(val: CurrentCustomType) -> Self {
            val
        }
    }
    
    #[derive(Versioned)]
    #[versioned(current = "test")]
    pub struct VersionedStruct {
        pub normal_field: String,
        
        #[versioned(v6 = "v6::CustomType")]
        pub custom_field: CurrentCustomType,
    }
    
    // Should generate v6 module with appropriate struct
    let current = VersionedStruct {
        normal_field: "test".to_string(),
        custom_field: CurrentCustomType { value: 100 },
    };
    
    assert_eq!(current.custom_field.value, 100);
}

// Test enum versioning
#[test]
fn test_enum_versioning() {
    #[derive(Versioned)]
    #[versioned(current = "example")]
    pub enum MyEnum {
        Variant1(u32),
        Variant2(String),
        Variant3,
    }
    
    let e = MyEnum::Variant1(42);
    match e {
        MyEnum::Variant1(val) => assert_eq!(val, 42),
        _ => panic!("Wrong variant"),
    }
}