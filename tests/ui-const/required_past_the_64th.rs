// A mark on the 65th field. The mask is a `u64` and has no bit for it, and a
// shift by 64 would wrap onto the first field's, crediting the wrong member.
// The refusal is an `assert!` in the mask's constant, since `macro_rules!`
// cannot count to 64, and that constant is only evaluated once a reader
// instantiates it, which is what `main` is for.
#[derive(Default)]
#[rustfmt::skip]
struct Wide {
    f0: u8, f1: u8, f2: u8, f3: u8, f4: u8, f5: u8, f6: u8, f7: u8, f8: u8, f9: u8, f10: u8,
    f11: u8, f12: u8, f13: u8, f14: u8, f15: u8, f16: u8, f17: u8, f18: u8, f19: u8, f20: u8,
    f21: u8, f22: u8, f23: u8, f24: u8, f25: u8, f26: u8, f27: u8, f28: u8, f29: u8, f30: u8,
    f31: u8, f32: u8, f33: u8, f34: u8, f35: u8, f36: u8, f37: u8, f38: u8, f39: u8, f40: u8,
    f41: u8, f42: u8, f43: u8, f44: u8, f45: u8, f46: u8, f47: u8, f48: u8, f49: u8, f50: u8,
    f51: u8, f52: u8, f53: u8, f54: u8, f55: u8, f56: u8, f57: u8, f58: u8, f59: u8, f60: u8,
    f61: u8, f62: u8, f63: u8, f64: u8,
}

structio::object!(Wide {
    f0, f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14, f15, f16, f17, f18, f19, f20,
    f21, f22, f23, f24, f25, f26, f27, f28, f29, f30, f31, f32, f33, f34, f35, f36, f37, f38, f39,
    f40, f41, f42, f43, f44, f45, f46, f47, f48, f49, f50, f51, f52, f53, f54, f55, f56, f57, f58,
    f59, f60, f61, f62, f63,
    #[required]
    f64,
});

fn main() {
    let _ = structio::from_str::<Wide>("{}");
}
