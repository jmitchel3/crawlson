fn main() {
    println!("cargo:rerun-if-env-changed=CRAWLSON_UPDATE_PUBLIC_KEY");
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET");
    println!("cargo:rustc-env=CRAWLSON_BUILD_TARGET={target}");
}
