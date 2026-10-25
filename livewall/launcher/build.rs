fn main() {
    let payload = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist/payload.zip");

    if !payload.exists() {
        panic!(
            "{} is missing. Run tools/build.ps1, which builds it first.",
            payload.display()
        );
    }

    println!("cargo:rerun-if-changed={}", payload.display());
}
