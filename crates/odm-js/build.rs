fn main() {
    // The framework is embedded via include_dir!, which cargo cannot see;
    // without this, editing framework JS leaves a stale snapshot in the
    // binary.
    println!("cargo::rerun-if-changed=../../framework");
}
