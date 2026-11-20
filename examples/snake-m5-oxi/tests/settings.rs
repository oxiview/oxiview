//! Both sources build the same inline settings and run through real UI dispatch.
use std::{error::Error, fs, path::Path};

#[test]
fn authored_settings_have_identical_keyboard_behavior_and_pixels() -> Result<(), Box<dyn Error>> {
    let output = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/xtask")
        .join(format!("snake-m5-parity-{}", std::process::id()));
    let builder = output.join("builder");
    let oxi = output.join("oxi");
    let config = snake::application::Config::default();
    let expected = snake_m5_oxi::replay::run_with_panel(config, &builder, None, true)?;
    let actual = snake_m5_oxi::replay::run(config, &oxi, None)?;
    assert_eq!(expected, actual);
    for name in actual.images {
        assert!(
            fs::read(builder.join(&name))? == fs::read(oxi.join(&name))?,
            "different rendered pixels in {name}"
        );
    }
    fs::remove_dir_all(output)?;
    Ok(())
}
