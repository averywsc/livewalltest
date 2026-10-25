fn main() {
    if !std::path::Path::new("../dist/payload.zip").exists() {
        panic!("dist/payload.zip is missing. Run tools/build.ps1, which builds it first.");
    }
    println!("cargo:rerun-if-changed=../dist/payload.zip");
}
