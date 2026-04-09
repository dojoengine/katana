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
use katana_primitives::class::{ContractClass, MaybeInvalidSierraContractAbi};
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

/// Extract the constructor signature from a Sierra class, returning `None` for
/// legacy classes, classes without an ABI, classes with a corrupted ABI, and
/// classes whose ABI doesn't declare a constructor.
pub fn extract_constructor(class: &ContractClass) -> Option<ConstructorAbi> {
    let abi = class.as_sierra()?.abi.as_ref()?;
    let contract = match abi {
        MaybeInvalidSierraContractAbi::Valid(c) => c,
        MaybeInvalidSierraContractAbi::Invalid(_) => return None,
    };
    parse_abi(contract).ok()?.constructor
}

/// Render a [`TypeNode`] as a short, human-friendly type string. Strips the
/// `core::...::` prefix from primitive/struct/enum names so the TUI doesn't
/// show `core::starknet::contract_address::ContractAddress` for every input.
pub fn pretty_type(node: &TypeNode) -> String {
    fn short(name: &str) -> String {
        // Take the segment after the last `::`, but tolerate generic suffixes
        // like `Span::<core::felt252>` by stripping the `<...>` first.
        let head = name.split_once('<').map(|(h, _)| h.trim_end_matches("::")).unwrap_or(name);
        head.rsplit("::").next().unwrap_or(name).to_string()
    }
    match node {
        TypeNode::Primitive { name } => short(name),
        TypeNode::Struct { name, .. } => short(name),
        TypeNode::Enum { name, .. } => short(name),
        TypeNode::Array { element, .. } => format!("Array<{}>", pretty_type(element)),
        TypeNode::Option { element, .. } => format!("Option<{}>", pretty_type(element)),
        TypeNode::Unknown { name } => short(name),
    }
}

/// Convert a free-form text input from a TUI/CLI into a [`serde_json::Value`]
/// suitable for [`to_calldata`].
///
/// The rules try to do the least surprising thing:
///
/// - Empty / whitespace input → `Value::Null` (so `Option<T>` users can leave the field blank to
///   mean "absent").
/// - Otherwise, try to parse as JSON first. This handles numbers, booleans, arrays (`[1, 2, 3]`),
///   and structs (`{"a": 1}`).
/// - On JSON parse failure, fall back to `Value::String(text)`. This is what lets users type bare
///   hex literals like `0x42` without quoting.
pub fn parse_text_value(text: &str) -> Value {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(trimmed.to_string()))
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
    // Several Cairo types are represented as structs in the ABI but should be
    // entered as a single value by the user. Treat them as primitives so the
    // `to_calldata` branches handle encoding transparently:
    //
    // - `u256`: two-member struct (low, high) — user enters one number, `split_u256` produces the
    //   two felts.
    // - `ByteArray`: three-member struct (data, pending_word, pending_word_len) — user enters a
    //   plain string, `encode_byte_array` chunks it.
    // - `ContractAddress`, `ClassHash`, `EthAddress`, `StorageAddress`: single- felt wrapper
    //   structs — user enters one hex/decimal value.
    if is_transparent_struct(type_str) {
        return TypeNode::Primitive { name: type_str.to_string() };
    }

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

/// Types that the ABI defines as structs but that the user should enter as a
/// single scalar value. Each of these either wraps a single felt (addresses,
/// hashes) or has custom encoding logic in [`to_calldata`] (u256, ByteArray).
fn is_transparent_struct(type_str: &str) -> bool {
    matches!(
        type_str,
        "core::integer::u256"
            | "core::byte_array::ByteArray"
            | "core::starknet::contract_address::ContractAddress"
            | "core::starknet::class_hash::ClassHash"
            | "core::starknet::eth_address::EthAddress"
            | "core::starknet::storage_access::StorageAddress"
    )
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
            "core::byte_array::ByteArray" => {
                let s = value.as_str().ok_or_else(|| AbiError::TypeMismatch {
                    ty: name.clone(),
                    message: "expected a string".to_string(),
                })?;
                Ok(encode_byte_array(s.as_bytes()))
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

/// Try to decode felts back into a human-readable text value for a given type.
///
/// Returns `(display_string, felts_consumed)` on success, or `None` if the
/// decoding can't be done (not enough felts, unsupported type, etc). This is
/// best-effort — it covers the common primitives, u256, ByteArray, and simple
/// structs so the TUI can pre-fill inputs when editing an existing deploy.
pub fn from_calldata(node: &TypeNode, felts: &[Felt]) -> Option<(String, usize)> {
    match node {
        TypeNode::Primitive { name } => match name.as_str() {
            "core::integer::u256" => {
                let (low, high) = (felts.first()?, felts.get(1)?);
                // Reconstruct the U256 from low + high.
                let low_u128: u128 = (*low).try_into().ok()?;
                let high_u128: u128 = (*high).try_into().ok()?;
                let value = U256::from(high_u128) << 128 | U256::from(low_u128);
                Some((format!("{value:#x}"), 2))
            }
            "core::byte_array::ByteArray" => {
                let num_chunks: u64 = (*felts.first()?).try_into().ok()?;
                let total = 1 + num_chunks as usize + 2; // len + chunks + pending + pending_len
                if felts.len() < total {
                    return None;
                }
                let mut bytes = Vec::new();
                for i in 0..num_chunks as usize {
                    let chunk = felts[1 + i].to_bytes_be();
                    // Each chunk is 31 bytes — the last 31 bytes of the 32-byte BE repr.
                    bytes.extend_from_slice(&chunk[1..]);
                }
                let pending_word = felts[1 + num_chunks as usize];
                let pending_len: u64 = felts[2 + num_chunks as usize].try_into().ok()?;
                if pending_len > 0 {
                    let pw_bytes = pending_word.to_bytes_be();
                    let start = 32 - pending_len as usize;
                    bytes.extend_from_slice(&pw_bytes[start..]);
                }
                let s = String::from_utf8(bytes).ok()?;
                Some((s, total))
            }
            "core::bool" => {
                // bool is an enum in Cairo, but if it was encoded as a primitive
                // it's a single felt: 0 = false, 1 = true.
                let f = felts.first()?;
                let display = if *f == Felt::ONE { "true" } else { "false" };
                Some((display.to_string(), 1))
            }
            _ => {
                // Single-felt primitive (felt252, u8..u128, ContractAddress, etc).
                let f = felts.first()?;
                Some((format!("{f:#x}"), 1))
            }
        },

        TypeNode::Struct { members, .. } => {
            let mut consumed = 0;
            let mut obj = serde_json::Map::new();
            for (member_name, member_ty) in members {
                let (val_str, n) = from_calldata(member_ty, &felts[consumed..])?;
                obj.insert(member_name.clone(), Value::String(val_str));
                consumed += n;
            }
            let json = serde_json::to_string(&Value::Object(obj)).ok()?;
            Some((json, consumed))
        }

        TypeNode::Enum { name, .. } => {
            if name == "core::bool" {
                let f = felts.first()?;
                let display = if *f == Felt::ONE { "true" } else { "false" };
                Some((display.to_string(), 1))
            } else {
                None // Can't decode arbitrary enums
            }
        }

        TypeNode::Option { element, .. } => {
            let tag = felts.first()?;
            if *tag == Felt::ONE {
                // None
                Some((String::new(), 1))
            } else {
                let (inner, n) = from_calldata(element, &felts[1..])?;
                Some((inner, 1 + n))
            }
        }

        TypeNode::Array { element, .. } => {
            let len: u64 = (*felts.first()?).try_into().ok()?;
            let mut consumed = 1;
            let mut items = Vec::new();
            for _ in 0..len {
                let (val_str, n) = from_calldata(element, &felts[consumed..])?;
                items.push(val_str);
                consumed += n;
            }
            let json = serde_json::to_string(&items).ok()?;
            Some((json, consumed))
        }

        TypeNode::Unknown { .. } => None,
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

/// Encode a byte slice into the Cairo `ByteArray` calldata format.
///
/// Layout: `[num_full_chunks, ...chunk_felts, pending_word, pending_word_len]`
///
/// Each full chunk is 31 bytes, big-endian encoded as a felt. The remaining
/// `len % 31` bytes go into `pending_word` (also big-endian), with
/// `pending_word_len` recording how many bytes that is.
fn encode_byte_array(bytes: &[u8]) -> Vec<Felt> {
    const CHUNK: usize = 31;

    let full_chunks = bytes.len() / CHUNK;
    let remainder = bytes.len() % CHUNK;

    let mut out = Vec::with_capacity(full_chunks + 3);

    // Number of full 31-byte words.
    out.push(Felt::from(full_chunks as u64));

    // Each full chunk as a big-endian felt.
    for i in 0..full_chunks {
        let chunk = &bytes[i * CHUNK..(i + 1) * CHUNK];
        out.push(Felt::from_bytes_be_slice(chunk));
    }

    // Pending word (the leftover bytes, big-endian).
    if remainder > 0 {
        let tail = &bytes[full_chunks * CHUNK..];
        out.push(Felt::from_bytes_be_slice(tail));
    } else {
        out.push(Felt::ZERO);
    }

    // Pending word length.
    out.push(Felt::from(remainder as u64));

    out
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
    fn u256_struct_in_abi_resolved_as_primitive() {
        // In a real Sierra ABI, u256 is defined as a struct with `low` and
        // `high` members. Verify that `resolve_type` short-circuits to
        // `Primitive` so the user can type a single number instead of a
        // JSON struct literal.
        let abi = make_contract(json!([
            {
                "type": "struct",
                "name": "core::integer::u256",
                "members": [
                    { "name": "low", "type": "core::integer::u128" },
                    { "name": "high", "type": "core::integer::u128" }
                ]
            },
            {
                "type": "constructor",
                "name": "constructor",
                "inputs": [{ "name": "amount", "type": "core::integer::u256" }]
            }
        ]));

        let parsed = parse_abi(&abi).unwrap();
        let ctor = parsed.constructor.unwrap();
        assert_eq!(ctor.inputs.len(), 1);
        match &ctor.inputs[0].ty {
            TypeNode::Primitive { name } => assert_eq!(name, "core::integer::u256"),
            other => panic!("expected Primitive for u256, got {other:?}"),
        }

        // End-to-end: the user types a single hex literal in the TUI and the
        // encoding produces [low, high].
        let calldata = to_calldata(&ctor.inputs[0].ty, &json!("0x1")).unwrap();
        assert_eq!(calldata, vec![Felt::ONE, Felt::ZERO]);
    }

    // ----- from_calldata round-trip tests ------------------------------------

    #[test]
    fn from_calldata_felt_round_trips() {
        let node = TypeNode::Primitive { name: "core::felt252".to_string() };
        let felts = [Felt::from(0x42u64)];
        let (display, consumed) = from_calldata(&node, &felts).unwrap();
        assert_eq!(display, "0x42");
        assert_eq!(consumed, 1);
    }

    #[test]
    fn from_calldata_u256_round_trips() {
        let node = TypeNode::Primitive { name: "core::integer::u256".to_string() };
        // 256 = low=0x100, high=0x0
        let felts = [Felt::from(0x100u64), Felt::ZERO];
        let (display, consumed) = from_calldata(&node, &felts).unwrap();
        assert_eq!(display, "0x100");
        assert_eq!(consumed, 2);
    }

    #[test]
    fn from_calldata_byte_array_round_trips() {
        let node = TypeNode::Primitive { name: "core::byte_array::ByteArray".to_string() };
        let encoded = to_calldata(&node, &json!("hello")).unwrap();
        let (display, consumed) = from_calldata(&node, &encoded).unwrap();
        assert_eq!(display, "hello");
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn from_calldata_bool_round_trips() {
        let node = TypeNode::Enum { name: "core::bool".to_string(), variants: vec![] };
        let (display, consumed) = from_calldata(&node, &[Felt::ONE]).unwrap();
        assert_eq!(display, "true");
        assert_eq!(consumed, 1);
        let (display, consumed) = from_calldata(&node, &[Felt::ZERO]).unwrap();
        assert_eq!(display, "false");
        assert_eq!(consumed, 1);
    }

    #[test]
    fn from_calldata_returns_none_on_insufficient_felts() {
        let node = TypeNode::Primitive { name: "core::integer::u256".to_string() };
        assert!(from_calldata(&node, &[]).is_none());
        assert!(from_calldata(&node, &[Felt::ONE]).is_none());
    }

    // ----- to_calldata tests ------------------------------------------------

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
    fn contract_address_struct_in_abi_resolved_as_primitive() {
        let abi = make_contract(json!([
            {
                "type": "struct",
                "name": "core::starknet::contract_address::ContractAddress",
                "members": [{ "name": "address", "type": "core::felt252" }]
            },
            {
                "type": "constructor",
                "name": "constructor",
                "inputs": [
                    { "name": "owner", "type": "core::starknet::contract_address::ContractAddress" }
                ]
            }
        ]));

        let parsed = parse_abi(&abi).unwrap();
        let ctor = parsed.constructor.unwrap();
        match &ctor.inputs[0].ty {
            TypeNode::Primitive { name } => {
                assert_eq!(name, "core::starknet::contract_address::ContractAddress")
            }
            other => panic!("expected Primitive for ContractAddress, got {other:?}"),
        }

        // User enters a hex address — encodes as a single felt.
        let addr = "0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec";
        let calldata = to_calldata(&ctor.inputs[0].ty, &json!(addr)).unwrap();
        assert_eq!(calldata.len(), 1);
        assert_eq!(calldata[0], Felt::from_str(addr).unwrap());
    }

    #[test]
    fn byte_array_struct_in_abi_resolved_as_primitive() {
        let abi = make_contract(json!([
            {
                "type": "struct",
                "name": "core::byte_array::ByteArray",
                "members": [
                    { "name": "data", "type": "core::array::Array::<core::bytes_31::bytes31>" },
                    { "name": "pending_word", "type": "core::felt252" },
                    { "name": "pending_word_len", "type": "core::integer::u32" }
                ]
            },
            {
                "type": "constructor",
                "name": "constructor",
                "inputs": [{ "name": "name", "type": "core::byte_array::ByteArray" }]
            }
        ]));

        let parsed = parse_abi(&abi).unwrap();
        let ctor = parsed.constructor.unwrap();
        match &ctor.inputs[0].ty {
            TypeNode::Primitive { name } => assert_eq!(name, "core::byte_array::ByteArray"),
            other => panic!("expected Primitive for ByteArray, got {other:?}"),
        }
    }

    #[test]
    fn calldata_byte_array_short_string() {
        // "hello" (5 bytes) fits entirely in the pending word.
        let node = TypeNode::Primitive { name: "core::byte_array::ByteArray".to_string() };
        let out = to_calldata(&node, &json!("hello")).unwrap();
        assert_eq!(
            out,
            vec![
                Felt::ZERO,                          // 0 full chunks
                Felt::from_bytes_be_slice(b"hello"), // pending_word
                Felt::from(5u64),                    // pending_word_len
            ]
        );
    }

    #[test]
    fn calldata_byte_array_long_string() {
        // 37 bytes = 1 full 31-byte chunk + 6 remaining.
        let node = TypeNode::Primitive { name: "core::byte_array::ByteArray".to_string() };
        let input = "Long string, more than 31 characters.";
        assert_eq!(input.len(), 37);

        let out = to_calldata(&node, &json!(input)).unwrap();
        assert_eq!(out.len(), 4); // 1 (len) + 1 (chunk) + 1 (pending) + 1 (pending_len)
        assert_eq!(out[0], Felt::from(1u64)); // 1 full chunk
        assert_eq!(out[1], Felt::from_bytes_be_slice(&input.as_bytes()[..31]));
        assert_eq!(out[2], Felt::from_bytes_be_slice(&input.as_bytes()[31..]));
        assert_eq!(out[3], Felt::from(6u64)); // 6 remaining bytes
    }

    #[test]
    fn calldata_byte_array_exact_31_bytes() {
        // Exactly 31 bytes = 1 full chunk, 0 pending.
        let node = TypeNode::Primitive { name: "core::byte_array::ByteArray".to_string() };
        let input = "abcdefghijklmnopqrstuvwxyz01234";
        assert_eq!(input.len(), 31);

        let out = to_calldata(&node, &json!(input)).unwrap();
        assert_eq!(out[0], Felt::from(1u64));
        assert_eq!(out[1], Felt::from_bytes_be_slice(input.as_bytes()));
        assert_eq!(out[2], Felt::ZERO); // empty pending
        assert_eq!(out[3], Felt::ZERO); // pending_len = 0
    }

    #[test]
    fn calldata_byte_array_empty() {
        let node = TypeNode::Primitive { name: "core::byte_array::ByteArray".to_string() };
        let out = to_calldata(&node, &json!("")).unwrap();
        assert_eq!(out, vec![Felt::ZERO, Felt::ZERO, Felt::ZERO]);
    }

    #[test]
    fn calldata_bool() {
        let node = TypeNode::Enum { name: "core::bool".to_string(), variants: vec![] };
        assert_eq!(to_calldata(&node, &json!(true)).unwrap(), vec![Felt::ONE]);
        assert_eq!(to_calldata(&node, &json!(false)).unwrap(), vec![Felt::ZERO]);
    }

    #[test]
    fn pretty_type_strips_core_prefixes() {
        assert_eq!(
            pretty_type(&TypeNode::Primitive { name: "core::integer::u256".to_string() }),
            "u256"
        );
        assert_eq!(
            pretty_type(&TypeNode::Struct {
                name: "core::starknet::contract_address::ContractAddress".to_string(),
                members: vec![],
            }),
            "ContractAddress"
        );
        assert_eq!(
            pretty_type(&TypeNode::Array {
                name: "core::array::Array::<core::felt252>".to_string(),
                element: Box::new(TypeNode::Primitive { name: "core::felt252".to_string() }),
            }),
            "Array<felt252>"
        );
        assert_eq!(
            pretty_type(&TypeNode::Option {
                name: "core::option::Option::<core::integer::u32>".to_string(),
                element: Box::new(TypeNode::Primitive { name: "core::integer::u32".to_string() }),
            }),
            "Option<u32>"
        );
        // Generic struct with `<...>` suffix should still strip cleanly.
        assert_eq!(
            pretty_type(&TypeNode::Struct {
                name: "core::array::Span::<core::felt252>".to_string(),
                members: vec![],
            }),
            "Span"
        );
    }

    #[test]
    fn parse_text_value_handles_blank_json_and_bare_hex() {
        assert_eq!(parse_text_value(""), Value::Null);
        assert_eq!(parse_text_value("   "), Value::Null);
        assert_eq!(parse_text_value("42"), json!(42));
        assert_eq!(parse_text_value("true"), json!(true));
        assert_eq!(parse_text_value(r#"[1,2,3]"#), json!([1, 2, 3]));
        assert_eq!(parse_text_value(r#"{"a":1}"#), json!({ "a": 1 }));
        // Bare hex literal isn't valid JSON; should fall back to a string.
        assert_eq!(parse_text_value("0x42"), Value::String("0x42".to_string()));
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
