#![crate_type = "lib"]

#[unsafe(no_mangle)]
pub extern "C" fn compute_rust(x: i32) -> i32 {
    x.wrapping_mul(1337).wrapping_add(7)
}