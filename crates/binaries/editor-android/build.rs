fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        // Native dependencies must resolve at link time, before an APK can ship.
        println!("cargo::rustc-link-arg-cdylib=-Wl,--no-undefined");
        println!("cargo::rerun-if-env-changed=CC");
        let compiler = std::env::var_os("CC").expect("Android NDK C compiler");
        let output = std::process::Command::new(compiler)
            .arg("--print-libgcc-file-name")
            .output()
            .expect("query NDK compiler runtime");
        assert!(output.status.success(), "NDK compiler runtime query failed");
        let library = String::from_utf8(output.stdout).expect("NDK compiler runtime path");
        let library = library.trim();
        assert!(
            std::path::Path::new(library).is_file(),
            "NDK compiler runtime is missing"
        );
        // Rust omits the C driver's default libraries. Native libffi/PCRE need
        // its instruction-cache flush implementation on ARM.
        println!("cargo::rustc-link-arg-cdylib={library}");
    }
}
