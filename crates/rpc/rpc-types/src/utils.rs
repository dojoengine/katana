use anyhow::Result;
use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
use katana_primitives::Felt;
pub use starknet::core::types::contract::CompiledClass;
use starknet::core::types::FlattenedSierraClass;

/// Compute the compiled class hash from the given [`FlattenedSierraClass`].
pub fn compiled_class_hash_from_flattened_sierra_class(
    contract_class: &FlattenedSierraClass,
) -> Result<Felt> {
    let contract_class = rpc_to_cairo_contract_class(contract_class)?;
    let casm = CasmContractClass::from_contract_class(contract_class, true, usize::MAX)?;
    let compiled_class: CompiledClass = serde_json::from_str(&serde_json::to_string(&casm)?)?;
    Ok(compiled_class.class_hash()?)
}

/// Converts `starknet-rs` RPC [FlattenedSierraClass] type to Cairo's
/// [ContractClass](cairo_lang_starknet::contract_class::ContractClass) type.
pub fn rpc_to_cairo_contract_class(
    contract_class: &FlattenedSierraClass,
) -> Result<cairo_lang_starknet_classes::contract_class::ContractClass, std::io::Error> {
    let value = serde_json::to_value(contract_class)?;

    Ok(cairo_lang_starknet_classes::contract_class::ContractClass {
        abi: serde_json::from_value(value["abi"].clone()).ok(),
        sierra_program: serde_json::from_value(value["sierra_program"].clone())?,
        entry_points_by_type: serde_json::from_value(value["entry_points_by_type"].clone())?,
        contract_class_version: serde_json::from_value(value["contract_class_version"].clone())?,
        sierra_program_debug_info: serde_json::from_value(
            value["sierra_program_debug_info"].clone(),
        )
        .ok(),
    })
}
