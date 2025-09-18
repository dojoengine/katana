use katana_db_versioned_derive::versioned;
use serde::{Deserialize, Serialize};

// Test that the attribute macro works correctly
#[versioned]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestStruct {
    pub field1: u32,
    
    #[versioned(v6 = "u64")]
    pub field2: String,
}

#[test]
fn test_attribute_macro_generates_struct() {
    // The struct should be available
    let test = TestStruct {
        field1: 42,
        field2: "hello".to_string(),
    };
    
    assert_eq!(test.field1, 42);
    assert_eq!(test.field2, "hello");
}

#[test] 
fn test_v6_module_generated() {
    // The v6 module should be generated with the versioned type
    let v6_test = v6::TestStruct {
        field1: 100,
        field2: 200u64,
    };
    
    assert_eq!(v6_test.field1, 100);
    assert_eq!(v6_test.field2, 200);
    
    // Test conversion from v6 to current
    let current: TestStruct = v6_test.into();
    assert_eq!(current.field1, 100);
    assert_eq!(current.field2, "200");  // u64 converts to String via Into trait
}