// Force recompilation when the embedded SP1 guest ELF changes
// include_bytes! does not track the file as a dependency on its own
fn main() {
    println!("cargo:rerun-if-changed=../sp1-program/elf/riscv32im-succinct-zkvm-elf");
}
