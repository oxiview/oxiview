//! Minimized input sequences also run under the ordinary stable verification gate.

#[path = "../../../fuzz/scenarios.rs"]
mod scenarios;

#[test]
fn unicode_and_native_session_seeds() {
    for bytes in [
        include_bytes!("../../../fuzz/seeds/editing/graphemes").as_slice(),
        include_bytes!("../../../fuzz/seeds/input_session/preedit").as_slice(),
    ] {
        scenarios::editing(bytes);
        scenarios::input_session(bytes);
    }
    for seed in 1..=96u32 {
        let mut state = seed;
        let bytes: Vec<_> = (0..160)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        scenarios::editing(&bytes);
        scenarios::input_session(&bytes);
    }
}
