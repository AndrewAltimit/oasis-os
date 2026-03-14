//! Custom getrandom backends for PSP (no native OS entropy source).
//!
//! Uses the PSP's hardware MT19937 PRNG via `sceKernelUtils`.

/// getrandom 0.2 custom backend (used by transitive deps like webpki).
pub(crate) mod psp_getrandom_v02 {
    use psp::sys::{
        sceKernelGetSystemTimeLow, sceKernelUtilsMt19937Init, sceKernelUtilsMt19937UInt,
    };

    fn psp_fill_random(buf: &mut [u8]) -> Result<(), getrandom_02::Error> {
        // SAFETY: MT19937 context is initialized by sceKernelUtilsMt19937Init
        // before any reads. Seed from system timer (user-mode safe).
        // mfc0 $9 (COP0 Count) is privileged on PSP Allegrex.
        unsafe {
            let mut ctx = core::mem::MaybeUninit::uninit();
            let seed = sceKernelGetSystemTimeLow() as u32;
            sceKernelUtilsMt19937Init(ctx.as_mut_ptr(), seed);
            let mut ctx = ctx.assume_init();
            for byte in buf.iter_mut() {
                *byte = (sceKernelUtilsMt19937UInt(&mut ctx) & 0xFF) as u8;
            }
        }
        Ok(())
    }

    getrandom_02::register_custom_getrandom!(psp_fill_random);
}

/// getrandom 0.3 custom backend (enabled via `--cfg getrandom_backend="custom"`
/// in `.cargo/config.toml`).
#[unsafe(no_mangle)]
pub(crate) unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    use psp::sys::{
        sceKernelGetSystemTimeLow, sceKernelUtilsMt19937Init, sceKernelUtilsMt19937UInt,
    };
    // SAFETY: MT19937 context is initialized by sceKernelUtilsMt19937Init
    // before any reads. Seed from system timer (user-mode safe).
    // mfc0 $9 (COP0 Count) is privileged on PSP Allegrex.
    unsafe {
        let mut ctx = core::mem::MaybeUninit::uninit();
        let seed = sceKernelGetSystemTimeLow() as u32;
        sceKernelUtilsMt19937Init(ctx.as_mut_ptr(), seed);
        let mut ctx = ctx.assume_init();
        for i in 0..len {
            *dest.add(i) = (sceKernelUtilsMt19937UInt(&mut ctx) & 0xFF) as u8;
        }
    }
    Ok(())
}
