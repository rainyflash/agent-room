fn main() {
    println!("cargo:rerun-if-changed=../../infra/migrations");
}
