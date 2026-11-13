//! Generated images verify comparison behavior without storing committed baselines.
#![allow(clippy::expect_used)]

use super::*;

fn gradient() -> RgbaImage {
    let pixels = (0..16u8)
        .flat_map(|y| (0..16u8).flat_map(move |x| [x * 16, y * 16, 128, 255]))
        .collect();
    RgbaImage::new(16, 16, pixels).expect("gradient")
}

#[test]
fn identical_gradient_passes() {
    let image = gradient();
    let (result, diff) = compare(&image, &image, &CompareConfig::default()).expect("compare");
    assert!(result.passed);
    assert_eq!(result.diff_pixels, 0);
    assert_eq!(result.max_channel_diff, 0);
    assert_eq!(&diff.pixels()[..4], &[191, 191, 223, 255]);
}

#[test]
fn tolerance_includes_alpha_and_ratio_boundary() {
    let expected = gradient();
    let mut actual = expected.clone();
    actual.pixels[3] -= 5;
    let config = CompareConfig {
        channel_tolerance: 5,
        allowed_diff_ratio: 0.0,
    };
    let (result, _) = compare(&actual, &expected, &config).expect("compare");
    assert!(result.passed);
    assert_eq!(result.diff_pixels, 0);
    assert_eq!(result.max_channel_diff, 5);
    let (result, diff) = compare(&actual, &expected, &CompareConfig::default()).expect("compare");
    assert!(!result.passed);
    assert_eq!(result.diff_pixels, 1);
    assert_eq!(result.diff_ratio, 1.0 / 256.0);
    assert_eq!(&diff.pixels()[..4], &[255, 0, 255, 255]);
    let config = CompareConfig {
        channel_tolerance: 0,
        allowed_diff_ratio: 1.0 / 256.0,
    };
    assert!(
        compare(&actual, &expected, &config)
            .expect("ratio boundary")
            .0
            .passed
    );
}

#[test]
fn dimensions_always_fail_even_with_full_tolerance() {
    let a = RgbaImage::new(2, 1, vec![0; 8]).expect("landscape image");
    let b = RgbaImage::new(1, 2, vec![0; 8]).expect("portrait image");
    let config = CompareConfig {
        channel_tolerance: 255,
        allowed_diff_ratio: 1.0,
    };
    let (result, diff) = compare(&a, &b, &config).expect("compare");
    assert!(!result.passed);
    assert!(!result.dimensions_match);
    assert_eq!(result.diff_pixels, 2);
    assert_eq!(result.max_channel_diff, 255);
    assert_eq!((diff.width(), diff.height()), (2, 2));
}

#[test]
fn invalid_inputs_are_errors() {
    assert!(RgbaImage::new(0, 1, vec![]).is_err());
    assert!(RgbaImage::new(2, 1, vec![0; 4]).is_err());
    assert!(RgbaImage::new(u32::MAX, u32::MAX, vec![]).is_err());
    let image = gradient();
    for ratio in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert!(
            compare(
                &image,
                &image,
                &CompareConfig {
                    channel_tolerance: 0,
                    allowed_diff_ratio: ratio
                }
            )
            .is_err()
        );
    }
}
