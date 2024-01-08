#!/usr/bin/env -S rust-script -c
//! You might need to chmod +x your script!
//! ```cargo
//! [dependencies.rust-script-ext]
//! git = "https://github.com/kurtlawrence/rust-script-ext"
//! rev = "47361b1a62272e6bf94ed849ec06c8df79f02362"
//! ```
// See <https://kurtlawrence.github.io/rust-script-ext/rust_script_ext/> for documentation
use rust_script_ext::prelude::*;

fn main() -> Result<()> {
    let arch = "arm-unknown-linux-musleabihf";
    cmd!(cargo: b, --release, -p pical, --target, {arch}).run()?;
    cmd!(cargo: b, --release, -p it8951-driver, --target, {arch}).run()?;
    cmd!(rsync: {format!("/tmp/pical/{arch}/release/pical")}, raspi-zerow:~/pical, -vzh).run()?;
    cmd!(rsync: {format!("/tmp/pical/{arch}/release/it8951-driver")}, raspi-zerow:~/it8951-driver, -vzh).run()?;
    println!("✅ Good to go!");
    Ok(())
}
