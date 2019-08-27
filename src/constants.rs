use crate::types::Byte;

pub const ZERO_BYTE: u8 = 0u8;
pub const HASH_LENGTH: usize  = 32;
pub const HASH_HEX_CHARS: usize  = 64;
pub const HEX_PREFIX_LENGTH: usize = 2;
pub const NUM_BITS_IN_NIBBLE: usize = 4;
pub const REQWEST_TIMEOUT_TIME: u64 = 5;
pub const NUM_NIBBLES_IN_BYTE: usize = 2;
pub const HIGH_NIBBLE_MASK: Byte = 15u8; // NOTE: 15u8 == [0,0,0,0,1,1,1,1]
pub const ADDRESS_LENGTH_CHARS: usize = 40;
pub static DOT_ENV_PATH: &'static str = "./.env";
pub static EMPTY_NODE: crate::types::Node<'static> = (&[], None);
pub static DEFAULT_ENDPOINT: &'static str = "http://localhost:8545/";
