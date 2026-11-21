//! Embeds an `__info_plist` in the macOS binary so an unbundled build (run
//! from a terminal or `cargo run`) declares Bluetooth usage: the device
//! pairing helper needs it for macOS to ask for access instead of killing it.
//! MeetNote.app's own Info.plist takes precedence inside the bundle.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let plist = out.join("Info.plist");
    std::fs::write(
        &plist,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.meetnote.app.cli</string>
  <key>CFBundleName</key>
  <string>MeetNote</string>
  <key>NSBluetoothAlwaysUsageDescription</key>
  <string>MeetNote searches for recording devices in pairing mode, such as the DJI Mic 3, so you can add them.</string>
</dict>
</plist>
"#,
    )
    .expect("write Info.plist");
    println!(
        "cargo:rustc-link-arg-bin=meetnote=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        plist.display()
    );
}
