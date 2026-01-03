#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub enum ContractClass {
    Class(cairo_lang_starknet_classes::contract_class::ContractClass),
    Legacy(katana_primitives::class::LegacyContractClass),
}

impl From<ContractClass> for katana_primitives::class::ContractClass {
    fn from(contract_class: ContractClass) -> Self {
        match contract_class {
            ContractClass::Legacy(class) => katana_primitives::class::ContractClass::Legacy(class),
            ContractClass::Class(class) => {
                katana_primitives::class::ContractClass::Class(class.into())
            }
        }
    }
}
