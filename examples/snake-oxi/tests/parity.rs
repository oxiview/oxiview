//! Uses identical inputs and rendering paths for both authored screens.

use snake::{application::Config, replay};
use std::{error::Error, fs, path::Path};

#[test]
fn replay_matches_every_image_and_counter() -> Result<(), Box<dyn Error>> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/xtask")
        .join(format!("snake-oxi-parity-{}", std::process::id()));
    let config = Config::default();
    let original = replay::run(config, &directory.join("rust"))?;
    let mut authored = replay::run_with_screen(config, &directory.join("oxi"), snake_oxi::screen)?;
    assert_eq!(original.images.len(), 6);
    assert_eq!(authored.images.len(), original.images.len());
    for (expected, actual) in original.images.iter().zip(&mut authored.images) {
        assert_eq!(
            Path::new(&expected.path).file_name(),
            Path::new(&actual.path).file_name()
        );
        assert!(
            fs::read(&expected.path)? == fs::read(&actual.path)?,
            "different rendered images at {} and {}",
            expected.path,
            actual.path
        );
        // Each executable keeps its own artifact directory.
        actual.path.clone_from(&expected.path);
    }
    assert_eq!(authored, original);
    fs::remove_dir_all(directory)?;
    Ok(())
}
