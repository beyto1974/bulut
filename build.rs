// Bakes the contents of the VERSION file (bumped by CI) into the binary.
fn main() {
    println!("cargo:rerun-if-changed=VERSION");
    let version = std::fs::read_to_string("VERSION")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|_| "0.0.0".to_string());
    println!("cargo:rustc-env=BULUT_VERSION={version}");
}
