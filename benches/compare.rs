//! Pixel comparison microbenchmarks; image preparation is outside measurement.
#![allow(clippy::expect_used)]

use oxiview_testing::{CompareConfig, RgbaImage, compare};

fn main() {
    divan::main();
}

#[divan::bench(args = [256, 1024], sample_count = 100, sample_size = 10)]
fn image_compare(bencher: divan::Bencher, size: u32) {
    let pixels = (0..size * size)
        .flat_map(|i| [(i % 256) as u8, 128, 64, 255])
        .collect();
    let image = RgbaImage::new(size, size, pixels).expect("valid image");
    let expected = image.clone();
    let config = CompareConfig::default();
    bencher.bench_local(|| {
        compare(
            divan::black_box(&image),
            divan::black_box(&expected),
            &config,
        )
        .expect("valid configuration")
    });
}
