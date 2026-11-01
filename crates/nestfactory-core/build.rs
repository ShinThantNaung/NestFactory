fn main() {
    // `include_dir!` does not track new template files; rebuild when the folder changes.
    println!("cargo:rerun-if-changed=templates");
}
