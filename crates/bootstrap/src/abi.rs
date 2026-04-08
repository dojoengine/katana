//! ABI -> typed AST -> calldata helpers.
//!
//! Rust port of `crates/explorer/ui/src/shared/utils/abi.ts`. Given a Sierra
//! contract ABI, [`parse_abi`] produces a [`ParsedAbi`] containing the
//! constructor and the read/write function lists with each input resolved to a
//! [`TypeNode`]. [`to_calldata`] then encodes a `serde_json::Value` against a
//! [`TypeNode`] into the felt sequence Starknet expects.
//!
//! JSON Schema generation from the TypeScript original is intentionally not
//! ported — no Rust caller needs it today.

use std::collections::HashMap;
use std::str::FromStr;

use cairo_lang_starknet_classes::abi::{Contract, Enum, Item, StateMutability, Struct};
use katana_primitives::utils::split_u256;
use katana_primitives::{Felt, U256};
use serde_json::Value;
use starknet::core::utils::get_selector_from_name;
use thiserror::Error;

/// A resolved Cairo type, the Rust mirror of the TS `TypeNode` discriminated
/// union.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeNode {
    Primitive { name: String },
    Struct { name: String, members: Vec<(String, TypeNode)> },
    Enum { name: String, variants: Vec<EnumVariantNode> },
    Option { name: String, element: Box<TypeNode> },
    Array { name: String, element: Box<TypeNode> },
    Unknown { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantNode {
    pub name: String,
    /// `None` for the unit variant (`()`), `Some(_)` otherwise.
    pub ty: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArgumentNode {
    pub name: String,
    pub ty: TypeNode,
}

#[derive(Debug, Clone)]
pub struct FunctionAbi {
    pub name: String,
    pub selector: Felt,
    pub state_mutability: StateMutability,
    /// Set when this function was discovered inside an `Item::Interface`.
    pub interface: Option<String>,
    pub inputs: Vec<ArgumentNode>,
}

#[derive(Debug, Clone)]
pub struct ConstructorAbi {
    pub name: String,
    pub inputs: Vec<ArgumentNode>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedAbi {
    pub constructor: Option<ConstructorAbi>,
    pub read_funcs: Vec<FunctionAbi>,
    pub write_funcs: Vec<FunctionAbi>,
}

#[derive(Debug, Error)]
pub enum AbiError {
    #[error("option enum `{0}` is missing required `Some` variant")]
    OptionMissingSome(String),

    #[error("type mismatch encoding `{ty}`: {message}")]
    TypeMismatch { ty: String, message: String },

    #[error("invalid integer literal for `{ty}`: {value}")]
    InvalidInteger { ty: String, value: String },

    #[error("invalid function name `{0}` for selector derivation")]
    InvalidSelectorName(String),
}

/// `true` if the function is a `view` (the cairo-lang ABI doesn't model
/// `pure`, so `view` is the only read variant).
pub fn is_read_function(f: &FunctionAbi) -> bool {
    matches!(f.state_mutability, StateMutability::View)
}

/// Walk an ABI and return the constructor + sorted read/write function lists.
pub fn parse_abi(abi: &Contract) -> Result<ParsedAbi, AbiError> {
    let owned_items = collect_items(abi);
    let items: Vec<&Item> = owned_items.iter().collect();
    let (structs, enums) = build_registries(&items);

    let mut out = ParsedAbi::default();

    for item in &items {
        match item {
            Item::Constructor(c) => {
                out.constructor = Some(ConstructorAbi {
                    name: c.name.clone(),
                    inputs: resolve_inputs(&c.inputs, &structs, &enums),
                });
            }
            Item::Function(f) => {
                let func = build_function(
                    &f.name,
                    &f.inputs,
                    f.state_mutability.clone(),
                    None,
                    &structs,
                    &enums,
                )?;
                push_function(&mut out, func);
            }
            Item::Interface(iface) => {
                for inner in &iface.items {
                    if let Item::Function(f) = inner {
                        let func = build_function(
                            &f.name,
                            &f.inputs,
                            f.state_mutability.clone(),
                            Some(iface.name.clone()),
                            &structs,
                            &enums,
                        )?;
                        push_function(&mut out, func);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(out)
}

fn push_function(out: &mut ParsedAbi, f: FunctionAbi) {
    if is_read_function(&f) {
        out.read_funcs.push(f);
    } else {
        out.write_funcs.push(f);
    }
}

fn build_function<'a>(
    name: &str,
    inputs: &[cairo_lang_starknet_classes::abi::Input],
    state_mutability: StateMutability,
    interface: Option<String>,
    structs: &HashMap<&'a str, &'a Struct>,
    enums: &HashMap<&'a str, &'a Enum>,
) -> Result<FunctionAbi, AbiError> {
    let selector = get_selector_from_name(name)
        .map_err(|_| AbiError::InvalidSelectorName(name.to_string()))?;
    Ok(FunctionAbi {
        name: name.to_string(),
        selector,
        state_mutability,
        interface,
        inputs: resolve_inputs(inputs, structs, enums),
    })
}

fn resolve_inputs<'a>(
    inputs: &[cairo_lang_starknet_classes::abi::Input],
    structs: &HashMap<&'a str, &'a Struct>,
    enums: &HashMap<&'a str, &'a Enum>,
) -> Vec<ArgumentNode> {
    inputs
        .iter()
        .map(|i| ArgumentNode { name: i.name.clone(), ty: resolve_type(&i.ty, structs, enums) })
        .collect()
}

/// Materialize an owned `Vec<Item>` from the contract.
///
/// `cairo_lang_starknet_classes::abi::Contract` only exposes a consuming
/// `IntoIterator`, so we round-trip through its `#[serde(transparent)]`
/// representation to borrow the items without consuming the input.
fn collect_items(abi: &Contract) -> Vec<Item> {
    serde_json::to_value(abi)
        .ok()
        .and_then(|v| serde_json::from_value::<Vec<Item>>(v).ok())
        .unwrap_or_default()
}

fn build_registries<'a>(
    items: &[&'a Item],
) -> (HashMap<&'a str, &'a Struct>, HashMap<&'a str, &'a Enum>) {
    let mut structs = HashMap::new();
    let mut enums = HashMap::new();
    for item in items {
        match item {
            Item::Struct(s) => {
                structs.insert(s.name.as_str(), s);
            }
            Item::Enum(e) => {
                enums.insert(e.name.as_str(), e);
            }
            _ => {}
        }
    }
    (structs, enums)
}

/// Resolve a type string to a [`TypeNode`], following the same precedence as
/// the TypeScript original.
fn resolve_type(
    type_str: &str,
    structs: &HashMap<&str, &Struct>,
    enums: &HashMap<&str, &Enum>,
) -> TypeNode {
    // Struct lookup, with the `core::array::Span` special case.
    if let Some(s) = structs.get(type_str) {
        if s.name.starts_with("core::array::Span") {
            // Spans wrap an `@core::array::Array::<T>` snapshot member.
            for member in &s.members {
                if member.name == "snapshot" && member.ty.contains("@core::array::Array") {
                    if let Some(inner) = slice_generic(&member.ty) {
                        return TypeNode::Array {
                            name: s.name.clone(),
                            element: Box::new(resolve_type(inner, structs, enums)),
                        };
                    }
                }
            }
            // Couldn't determine — fall back to a felt252 array, matching TS.
            return TypeNode::Array {
                name: s.name.clone(),
                element: Box::new(TypeNode::Primitive { name: "felt252".to_string() }),
            };
        }

        let members = s
            .members
            .iter()
            .map(|m| (m.name.clone(), resolve_type(&m.ty, structs, enums)))
            .collect();
        return TypeNode::Struct { name: s.name.clone(), members };
    }

    // Enum lookup, with the `core::option::Option` special case.
    if let Some(e) = enums.get(type_str) {
        if e.name.starts_with("core::option::Option") {
            // The `Some` variant carries the inner type.
            let some = e.variants.iter().find(|v| v.name == "Some");
            return match some {
                Some(v) => TypeNode::Option {
                    name: e.name.clone(),
                    element: Box::new(resolve_type(&v.ty, structs, enums)),
                },
                None => TypeNode::Unknown { name: e.name.clone() },
            };
        }

        let variants = e
            .variants
            .iter()
            .map(|v| EnumVariantNode {
                name: v.name.clone(),
                ty: if v.ty == "()" { None } else { Some(v.ty.clone()) },
            })
            .collect();
        return TypeNode::Enum { name: e.name.clone(), variants };
    }

    // Bare `core::array::Array::<T>` / `core::array::Span::<T>`.
    if (type_str.contains("core::array::Array") || type_str.contains("core::array::Span"))
        && type_str.contains('<')
        && type_str.ends_with('>')
    {
        if let Some(inner) = slice_generic(type_str) {
            return TypeNode::Array {
                name: inner.to_string(),
                element: Box::new(resolve_type(inner, structs, enums)),
            };
        }
    }

    TypeNode::Primitive { name: type_str.to_string() }
}

/// Extract `T` from `...<T>`. Returns `None` if there are no angle brackets.
fn slice_generic(s: &str) -> Option<&str> {
    let start = s.find('<')? + 1;
    let end = s.rfind('>')?;
    if start >= end {
        None
    } else {
        Some(&s[start..end])
    }
}

/// Encode a JSON value against a [`TypeNode`] into Starknet calldata.
pub fn to_calldata(node: &TypeNode, value: &Value) -> Result<Vec<Felt>, AbiError> {
    match node {
        TypeNode::Primitive { name } => match name.as_str() {
            "core::integer::u256" => {
                let big = parse_u256(value).ok_or_else(|| AbiError::InvalidInteger {
                    ty: name.clone(),
                    value: value.to_string(),
                })?;
                let (low, high) = split_u256(big);
                Ok(vec![low, high])
            }
            _ => {
                let f = parse_felt(value).ok_or_else(|| AbiError::InvalidInteger {
                    ty: name.clone(),
                    value: value.to_string(),
                })?;
                Ok(vec![f])
            }
        },

        TypeNode::Struct { name, members } => {
            // Mirror the TS branch that accepts a JSON-encoded string.
            let owned;
            let obj_value: &Value = if let Value::String(s) = value {
                owned = serde_json::from_str::<Value>(s).map_err(|e| AbiError::TypeMismatch {
                    ty: name.clone(),
                    message: format!("expected an object or JSON-encoded object: {e}"),
                })?;
                &owned
            } else {
                value
            };

            let obj = obj_value.as_object().ok_or_else(|| AbiError::TypeMismatch {
                ty: name.clone(),
                message: "expected an object".to_string(),
            })?;

            let mut out = Vec::new();
            for (member_name, member_ty) in members {
                let member_value = obj.get(member_name).unwrap_or(&Value::Null);
                out.extend(to_calldata(member_ty, member_value)?);
            }
            Ok(out)
        }

        TypeNode::Enum { name, .. } => {
            // Only `core::bool` is encoded; matches the TS source which punts
            // on every other enum.
            // TODO: full enum encoding (variant index + payload).
            if name == "core::bool" {
                let b = value.as_bool().ok_or_else(|| AbiError::TypeMismatch {
                    ty: name.clone(),
                    message: "expected a boolean".to_string(),
                })?;
                Ok(vec![if b { Felt::ONE } else { Felt::ZERO }])
            } else {
                Ok(Vec::new())
            }
        }

        TypeNode::Option { element, .. } => {
            // Bug-for-bug port: TS uses `value ? [0, ...inner] : [1]`, i.e.
            // present == 0, absent == 1. Preserve that here so the encoding
            // matches the explorer UI.
            if value.is_null() {
                Ok(vec![Felt::ONE])
            } else {
                let mut out = vec![Felt::ZERO];
                out.extend(to_calldata(element, value)?);
                Ok(out)
            }
        }

        TypeNode::Array { name, element } => {
            let arr = value.as_array().ok_or_else(|| AbiError::TypeMismatch {
                ty: name.clone(),
                message: "expected an array".to_string(),
            })?;
            let mut out = Vec::with_capacity(arr.len() + 1);
            out.push(Felt::from(arr.len() as u64));
            for elem in arr {
                out.extend(to_calldata(element, elem)?);
            }
            Ok(out)
        }

        TypeNode::Unknown { .. } => Ok(Vec::new()),
    }
}

fn parse_felt(value: &Value) -> Option<Felt> {
    match value {
        Value::String(s) => Felt::from_str(s).ok(),
        Value::Number(n) => n.as_u64().map(Felt::from).or_else(|| n.as_i64().map(Felt::from)),
        Value::Bool(b) => Some(if *b { Felt::ONE } else { Felt::ZERO }),
        _ => None,
    }
}

fn parse_u256(value: &Value) -> Option<U256> {
    match value {
        Value::String(s) => U256::from_str(s).ok(),
        Value::Number(n) => n.as_u128().map(U256::from).or_else(|| n.as_u64().map(U256::from)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn make_contract(items: Value) -> Contract {
        serde_json::from_value(items).expect("valid abi json")
    }

    #[test]
    fn parse_abi_finds_constructor_and_splits_funcs() {
        let abi = make_contract(json!([
            {
                "type": "constructor",
                "name": "constructor",
                "inputs": [
                    { "name": "a", "type": "core::felt252" },
                    { "name": "b", "type": "core::integer::u32" }
                ]
            },
            {
                "type": "function",
                "name": "get_value",
                "inputs": [],
                "outputs": [{ "type": "core::felt252" }],
                "state_mutability": "view"
            },
            {
                "type": "function",
                "name": "set_value",
                "inputs": [{ "name": "v", "type": "core::felt252" }],
                "outputs": [],
                "state_mutability": "external"
            },
            {
                "type": "interface",
                "name": "IFoo",
                "items": [
                    {
                        "type": "function",
                        "name": "iface_view",
                        "inputs": [],
                        "outputs": [{ "type": "core::felt252" }],
                        "state_mutability": "view"
                    }
                ]
            }
        ]));

        let parsed = parse_abi(&abi).unwrap();

        let ctor = parsed.constructor.expect("constructor present");
        assert_eq!(ctor.name, "constructor");
        assert_eq!(ctor.inputs.len(), 2);
        assert_eq!(ctor.inputs[0].name, "a");

        assert_eq!(parsed.read_funcs.len(), 2);
        assert_eq!(parsed.write_funcs.len(), 1);
        assert_eq!(parsed.write_funcs[0].name, "set_value");

        let iface_fn = parsed.read_funcs.iter().find(|f| f.name == "iface_view").unwrap();
        assert_eq!(iface_fn.interface.as_deref(), Some("IFoo"));

        // Selector check.
        let expected = get_selector_from_name("set_value").unwrap();
        assert_eq!(parsed.write_funcs[0].selector, expected);
    }

    #[test]
    fn resolves_span_struct_as_array() {
        let abi = make_contract(json!([
            {
                "type": "struct",
                "name": "core::array::Span::<core::felt252>",
                "members": [
                    { "name": "snapshot", "type": "@core::array::Array::<core::felt252>" }
                ]
            },
            {
                "type": "constructor",
                "name": "constructor",
                "inputs": [
                    { "name": "xs", "type": "core::array::Span::<core::felt252>" }
                ]
            }
        ]));

        let parsed = parse_abi(&abi).unwrap();
        let input = &parsed.constructor.unwrap().inputs[0];
        match &input.ty {
            TypeNode::Array { element, .. } => match element.as_ref() {
                TypeNode::Primitive { name } => assert_eq!(name, "core::felt252"),
                other => panic!("unexpected element: {other:?}"),
            },
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn resolves_option_enum() {
        let abi = make_contract(json!([
            {
                "type": "enum",
                "name": "core::option::Option::<core::felt252>",
                "variants": [
                    { "name": "Some", "type": "core::felt252" },
                    { "name": "None", "type": "()" }
                ]
            },
            {
                "type": "constructor",
                "name": "constructor",
                "inputs": [
                    { "name": "maybe", "type": "core::option::Option::<core::felt252>" }
                ]
            }
        ]));

        let parsed = parse_abi(&abi).unwrap();
        let input = &parsed.constructor.unwrap().inputs[0];
        match &input.ty {
            TypeNode::Option { element, .. } => match element.as_ref() {
                TypeNode::Primitive { name } => assert_eq!(name, "core::felt252"),
                other => panic!("unexpected element: {other:?}"),
            },
            other => panic!("expected option, got {other:?}"),
        }
    }

    #[test]
    fn resolves_nested_struct() {
        let abi = make_contract(json!([
            {
                "type": "struct",
                "name": "Inner",
                "members": [{ "name": "x", "type": "core::felt252" }]
            },
            {
                "type": "struct",
                "name": "Outer",
                "members": [{ "name": "inner", "type": "Inner" }]
            },
            {
                "type": "constructor",
                "name": "constructor",
                "inputs": [{ "name": "o", "type": "Outer" }]
            }
        ]));

        let parsed = parse_abi(&abi).unwrap();
        let input = &parsed.constructor.unwrap().inputs[0];
        match &input.ty {
            TypeNode::Struct { name, members } => {
                assert_eq!(name, "Outer");
                assert_eq!(members.len(), 1);
                assert_eq!(members[0].0, "inner");
                match &members[0].1 {
                    TypeNode::Struct { name, members } => {
                        assert_eq!(name, "Inner");
                        assert_eq!(members.len(), 1);
                    }
                    other => panic!("expected nested struct, got {other:?}"),
                }
            }
            other => panic!("expected struct, got {other:?}"),
        }
    }

    #[test]
    fn calldata_u256_hex_decimal_and_number() {
        let node = TypeNode::Primitive { name: "core::integer::u256".to_string() };

        let hex = to_calldata(&node, &json!("0x100000000000000000000000000000001")).unwrap();
        assert_eq!(hex, vec![Felt::from(1u128), Felt::from(1u128)]);

        let dec = to_calldata(&node, &json!("4")).unwrap();
        assert_eq!(dec, vec![Felt::from(4u128), Felt::ZERO]);

        let num = to_calldata(&node, &json!(42u64)).unwrap();
        assert_eq!(num, vec![Felt::from(42u128), Felt::ZERO]);
    }

    #[test]
    fn calldata_struct_flattens() {
        let node = TypeNode::Struct {
            name: "S".to_string(),
            members: vec![
                ("a".to_string(), TypeNode::Primitive { name: "core::integer::u32".to_string() }),
                ("b".to_string(), TypeNode::Primitive { name: "core::felt252".to_string() }),
            ],
        };

        let from_obj = to_calldata(&node, &json!({ "a": 7, "b": "0xabc" })).unwrap();
        assert_eq!(from_obj, vec![Felt::from(7u64), Felt::from(0xabcu64)]);

        // The TS branch that accepts a JSON-encoded string.
        let from_str = to_calldata(&node, &json!(r#"{"a":7,"b":"0xabc"}"#)).unwrap();
        assert_eq!(from_str, vec![Felt::from(7u64), Felt::from(0xabcu64)]);
    }

    #[test]
    fn calldata_array() {
        let node = TypeNode::Array {
            name: "core::array::Array::<core::felt252>".to_string(),
            element: Box::new(TypeNode::Primitive { name: "core::felt252".to_string() }),
        };
        let out = to_calldata(&node, &json!([1, 2, 3])).unwrap();
        assert_eq!(
            out,
            vec![Felt::from(3u64), Felt::from(1u64), Felt::from(2u64), Felt::from(3u64)]
        );
    }

    #[test]
    fn calldata_option_present_and_absent() {
        let node = TypeNode::Option {
            name: "core::option::Option::<core::felt252>".to_string(),
            element: Box::new(TypeNode::Primitive { name: "core::felt252".to_string() }),
        };

        let present = to_calldata(&node, &json!("0x5")).unwrap();
        assert_eq!(present, vec![Felt::ZERO, Felt::from(5u64)]);

        let absent = to_calldata(&node, &Value::Null).unwrap();
        assert_eq!(absent, vec![Felt::ONE]);
    }

    #[test]
    fn calldata_bool() {
        let node = TypeNode::Enum { name: "core::bool".to_string(), variants: vec![] };
        assert_eq!(to_calldata(&node, &json!(true)).unwrap(), vec![Felt::ONE]);
        assert_eq!(to_calldata(&node, &json!(false)).unwrap(), vec![Felt::ZERO]);
    }

    #[test]
    fn is_read_function_view() {
        let f = FunctionAbi {
            name: "x".to_string(),
            selector: Felt::ZERO,
            state_mutability: StateMutability::View,
            interface: None,
            inputs: vec![],
        };
        assert!(is_read_function(&f));

        let g = FunctionAbi { state_mutability: StateMutability::External, ..f };
        assert!(!is_read_function(&g));
    }
}
