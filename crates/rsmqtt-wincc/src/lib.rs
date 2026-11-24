//! WinCC OA driver logic of rsmqtt, independent of the WinCC OA API:
//! the address syntax ([`address`]), JSON paths ([`json`]), the Sparkplug B
//! host application ([`sparkplug`]), the redundancy names ([`redu`]) and the
//! settings of a broker connection ([`settings`]) and its addresses and
//! message routing ([`router`]), and payload text to numbers ([`values`]).
//! The driver (C++, ComDrv) calls them through the C ABI in [`ffi`]
//! (`include/rsmqtt_wincc.h`, C++ wrapper `include/rsmqtt_wincc.hpp`).
//!
//! The static library also carries the C ABI of the rsmqtt client, so the
//! driver links one Rust library.

// the rsmqtt C ABI goes into this static library
extern crate rsmqtt;

// only the json_bench example uses it (feature bench-sonic)
#[cfg(feature = "bench-sonic")]
use sonic_rs as _;

pub mod json;
pub mod address;
pub mod redu;
pub mod sparkplug;
pub mod settings;
pub mod router;
pub mod values;
pub mod ffi;
