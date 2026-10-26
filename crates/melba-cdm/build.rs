fn main() {
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include("third_party/cdm-api")
        .include("shim")
        .file("shim/cdm_shim.cc")
        .compile("melbacdm_shim");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rerun-if-changed=shim/cdm_shim.cc");
    println!("cargo:rerun-if-changed=shim/melba_cdm.h");
    println!("cargo:rerun-if-changed=third_party/cdm-api/content_decryption_module.h");
    println!("cargo:rerun-if-changed=third_party/cdm-api/content_decryption_module_ext.h");
    println!("cargo:rerun-if-changed=third_party/cdm-api/content_decryption_module_export.h");
}
