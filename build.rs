use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=AXON_LLVM_DIR");

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(d) = env::var("AXON_LLVM_DIR") {
        candidates.push(PathBuf::from(d));
    }
    // repo-local toolchain layout: <repo>/LLVM (also used when the crate is a workspace member)
    candidates.push(manifest.join("LLVM"));
    if let Some(parent) = manifest.parent() {
        candidates.push(parent.join("LLVM"));
    }
    candidates.push(PathBuf::from(r"C:\Program Files\LLVM"));

    let found = candidates
        .iter()
        .find(|p| p.join("lib").join("LLVM-C.lib").exists());

    match found {
        Some(dir) => {
            println!("cargo:rustc-link-search=native={}", dir.join("lib").display());
            println!("cargo:rustc-link-lib=dylib=LLVM-C");

            // Copy LLVM-C.dll next to every artifact dir so cargo run/test/examples
            // can load it without PATH setup.
            let dll = dir.join("bin").join("LLVM-C.dll");
            if let Ok(out) = env::var("OUT_DIR") {
                // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
                if let Some(profile_dir) = PathBuf::from(&out).ancestors().nth(3) {
                    for sub in ["", "deps", "examples"] {
                        let dir = if sub.is_empty() {
                            profile_dir.to_path_buf()
                        } else {
                            profile_dir.join(sub)
                        };
                        if dir.is_dir() {
                            let _ = fs::copy(&dll, dir.join("LLVM-C.dll"));
                        }
                    }
                }
            }
        }
        None => panic!(
            "LLVM-C.lib not found. Searched: {:?}. \
             Set AXON_LLVM_DIR to an LLVM install containing lib/LLVM-C.lib and bin/LLVM-C.dll.",
            candidates
        ),
    }
}
