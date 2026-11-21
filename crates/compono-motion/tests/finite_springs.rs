//! 5-3: a spring gives finite values whatever it is made of.

use compono_motion::{Spring, Timing};
use proptest::prelude::*;

/// Every kind of `f32` a caller can pass, ordinary and not.
fn any_value() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(f32::NAN),
        Just(f32::INFINITY),
        Just(f32::NEG_INFINITY),
        Just(0.0),
        Just(-0.0),
        Just(f32::MAX),
        Just(f32::MIN_POSITIVE),
        Just(1e12),
        -1e6_f32..1e6,
        0.0_f32..1e9,
        proptest::num::f32::ANY,
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        if cfg!(miri) { 8 } else { 2000 }
    ))]

    #[test]
    fn every_frame_of_any_spring_is_finite(
        damping_ratio in any_value(),
        stiffness in any_value(),
        velocity in -100.0_f32..100.0,
        t in 0.0_f32..12.0,
    ) {
        // Fields set directly, past the range `Spring::new` brings them into.
        let mut spring = Spring::MATERIAL_SPATIAL;
        spring.damping_ratio = damping_ratio;
        spring.stiffness = stiffness;
        let (x, v) = spring.position(t, velocity);
        prop_assert!(x.is_finite() && v.is_finite(), "{spring:?} at {t}: ({x}, {v})");
        prop_assert!(spring.settle_time().is_finite());
        let (progress, _done) = Timing::Spring(spring).progress(t);
        prop_assert!(progress.is_finite(), "{spring:?} at {t}: {progress}");
    }
}

#[test]
fn a_very_stiff_spring_gives_no_nan() {
    let mut stiff = Spring::MATERIAL_SPATIAL;
    stiff.stiffness = 1e12;
    let nan = (0..10_000)
        .filter(|frame| !stiff.position(*frame as f32 / 1000.0, 0.0).0.is_finite())
        .count();
    assert_eq!(nan, 0, "frames that were not finite");
}
