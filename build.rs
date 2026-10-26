fn main() {
    println!("cargo:rerun-if-changed=packaging/wrapper-auth.c");
    println!("cargo:rerun-if-changed=packaging/process-supervisor.c");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    // No NDK, C++ runtime, or Apple SDK at build time. Like argv-scrub.so,
    // this freestanding C library resolves libc against its eventual host.
    let compiler = cc::Build::new().get_compiler();
    let status = compiler
        .to_command()
        .args([
            "-O2",
            "-fPIC",
            "-shared",
            "-nostdlib",
            // The real payload cannot resolve libc symbols for a preload, so
            // the bridge must not import any: no stack protector, no builtins,
            // and its own syscalls/symbol lookup (see packaging/wrapper-auth.c).
            "-ffreestanding",
            "-fno-builtin",
            "-fno-stack-protector",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wl,--hash-style=sysv",
            "-o",
        ])
        .arg(output.join("melba-auth.so"))
        .arg("packaging/wrapper-auth.c")
        .status()
        .expect("compile local authentication bridge");
    assert!(
        status.success(),
        "cannot build the local authentication bridge"
    );
    let status = compiler
        .to_command()
        .args(["-O2", "-fPIE", "-pie", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(output.join("melba-supervisor"))
        .arg("packaging/process-supervisor.c")
        .status()
        .expect("compile host process supervisor");
    assert!(status.success(), "cannot build host process supervisor");
}
