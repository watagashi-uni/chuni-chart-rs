use sha2::{Digest, Sha256};
use std::{env, fs};

fn main() {
    // Content-based build identity invalidates validators after a renderer,
    // dependency, or embedded-asset change, including local untagged builds.
    let mut hash = Sha256::new();
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "src/chart.rs",
        "src/judgement.rs",
        "src/layout.rs",
        "src/render.rs",
        "src/scroll.rs",
        "assets/DejaVuSans.ttf",
    ] {
        println!("cargo:rerun-if-changed={path}");
        let data = fs::read(path).expect("Cannot fingerprint renderer source");
        hash.update((data.len() as u64).to_le_bytes());
        hash.update(data);
    }
    hash.update(env::var("TARGET").expect("Cargo TARGET missing"));
    println!("cargo:rustc-env=RENDERER_FINGERPRINT={:x}", hash.finalize());
}
