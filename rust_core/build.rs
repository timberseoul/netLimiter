use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    // OUT_DIR is like target/debug/build/<pkg>/out
    // We need to go up 3 levels to reach target/debug/ or target/release/
    let target_dir = Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .expect("Failed to find target directory");

    let libs_dir = Path::new("libs");

    let files = ["WinDivert.dll", "WinDivert64.sys"];

    for file in &files {
        let src = libs_dir.join(file);
        let dst = target_dir.join(file);
        if src.exists() {
            fs::copy(&src, &dst).unwrap_or_else(|e| {
                panic!("Failed to copy {} to {}: {}", src.display(), dst.display(), e)
            });
            println!("cargo:warning=Copied {} to {}", file, dst.display());
        } else {
            panic!("Required file not found: {}", src.display());
        }
    }

    // Re-run build script if the libs directory changes
    println!("cargo:rerun-if-changed=libs/WinDivert.dll");
    println!("cargo:rerun-if-changed=libs/WinDivert64.sys");
}
