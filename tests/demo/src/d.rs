#![crate_type = "lib"]

#[no_mangle]
pub extern "C" fn compute_rust(x: i32) -> i32 {
    x.wrapping_mul(1337).wrapping_add(7)
}