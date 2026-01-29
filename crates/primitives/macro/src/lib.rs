#![deny(missing_docs)]

//! Procedural macros for the `katana-primitives` crate.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use starknet_types_core::felt::{Felt, NonZeroFelt};
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, LitStr, Token};

/// 2 ** 251 - 256
///
/// Valid storage addresses should satisfy `address + offset < 2**251` where `offset <
/// 256` and `address < ADDR_BOUND`.
const ADDR_BOUND: NonZeroFelt = NonZeroFelt::from_raw([
    576459263475590224,
    18446744073709255680,
    160989183,
    18446743986131443745,
]);

const DEFAULT_CRATE_PATH: &str = "::katana_primitives";

/// Input for the `felt!` and `address!` macros.
///
/// Supports two forms:
/// - `felt!("0x1234")` - uses default crate path `::katana_primitives`
/// - `felt!("0x1234", crate)` - uses custom crate path
struct MacroInput {
    value: LitStr,
    crate_path: String,
}

impl Parse for MacroInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let value: LitStr = input.parse()?;

        let crate_path = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            // Parse remaining tokens as the crate path
            input.parse::<TokenStream2>()?.to_string()
        } else {
            DEFAULT_CRATE_PATH.to_string()
        };

        Ok(MacroInput { value, crate_path })
    }
}

fn parse_felt(s: &str) -> Felt {
    if s.starts_with("0x") || s.starts_with("0X") {
        Felt::from_hex(s).expect("invalid Felt hex value")
    } else {
        Felt::from_dec_str(s).expect("invalid Felt decimal value")
    }
}

/// Defines a compile-time constant for a field element from its decimal or hexadecimal
/// representation.
///
/// # Examples
///
/// ```ignore
/// use katana_primitives::felt;
///
/// // From hexadecimal (uses default crate path)
/// let hex_felt = felt!("0x1234");
///
/// // From decimal
/// let dec_felt = felt!("42");
///
/// // With custom crate path (for use inside katana-primitives itself)
/// let internal_felt = felt!("0x1234", crate);
/// ```
#[proc_macro]
pub fn felt(input: TokenStream) -> TokenStream {
    let MacroInput { value, crate_path } = parse_macro_input!(input as MacroInput);
    let felt_value = parse_felt(&value.value());
    let felt_raw = felt_value.to_raw();

    format!(
        "{}::Felt::from_raw([{}, {}, {}, {}])",
        crate_path, felt_raw[0], felt_raw[1], felt_raw[2], felt_raw[3],
    )
    .parse()
    .unwrap()
}

/// Defines a compile-time constant for a contract address from its decimal or hexadecimal
/// representation.
///
/// The address is normalized (i.e., `address % ADDR_BOUND`) at compile time.
///
/// # Examples
///
/// ```ignore
/// use katana_primitives::address;
///
/// // From hexadecimal (uses default crate path)
/// const MY_CONTRACT: ContractAddress = address!("0x1234");
///
/// // From decimal
/// const OTHER_CONTRACT: ContractAddress = address!("42");
///
/// // With custom crate path (for use inside katana-primitives itself)
/// const INTERNAL: ContractAddress = address!("0x1234", crate);
/// ```
#[proc_macro]
pub fn address(input: TokenStream) -> TokenStream {
    let MacroInput { value, crate_path } = parse_macro_input!(input as MacroInput);
    let felt_value = parse_felt(&value.value());

    // Normalize the address: address % ADDR_BOUND
    let normalized = felt_value.mod_floor(&ADDR_BOUND);
    let felt_raw = normalized.to_raw();

    format!(
        "{}::ContractAddress::from_raw([{}, {}, {}, {}])",
        crate_path, felt_raw[0], felt_raw[1], felt_raw[2], felt_raw[3],
    )
    .parse()
    .unwrap()
}
