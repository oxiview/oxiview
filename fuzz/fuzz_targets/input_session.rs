#![no_main]
use libfuzzer_sys::fuzz_target;
#[path = "../scenarios.rs"]
mod scenarios;
fuzz_target!(|bytes: &[u8]| scenarios::input_session(bytes));
