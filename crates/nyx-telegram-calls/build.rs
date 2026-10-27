fn main() {
    println!("cargo:rerun-if-changed=native/ntgcalls.h");
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../target/native/ntgcalls/lib");
    println!("cargo:rustc-env=PULSE_NTG_LIBRARY={}", root.join("libntgcalls.so").display());
    bindgen::Builder::default()
        .header("native/ntgcalls.h")
        .allowlist_function("ntg_.*")
        .allowlist_type("ntg_.*")
        .allowlist_var("NTG_.*")
        .prepend_enum_name(false)
        .derive_default(true)
        .dynamic_library_name("Native")
        .dynamic_link_require_all(true)
        .wrap_unsafe_ops(true)
        .generate()
        .expect("NTgCalls C API")
        .write_to_file(std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("native.rs"))
        .unwrap();
}
