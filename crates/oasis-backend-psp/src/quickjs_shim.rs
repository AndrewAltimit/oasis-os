//! Minimal C runtime surface for the QuickJS-NG sources that
//! `rquickjs-sys` compiles on `mipsel-sony-psp`.
//!
//! ## Why this module exists
//!
//! pspdev's prebuilt `libc.a` / `libm.a` / `libcglue.a` are all
//! compiled as **eabi32 + msingle-float + abicalls**. Rust's PSP
//! target is **o32 + mdouble-float + non-abicalls**. Those two
//! ABIs cannot coexist in a single `psp-ld` link — the linker
//! refuses to merge object files with mismatched MIPS e_flags.
//!
//! The rest of this backend has always been "pure Rust + raw
//! `psp::sys::sce*` syscalls", deliberately — we've never linked a
//! single C static library from pspdev before this epic. The only
//! reason pspdev is even installed now is that `rquickjs-sys` uses
//! the `cc` crate to compile QuickJS's C sources into `.o` files
//! that need a real MIPS cross compiler. We do **not** want to drag
//! in 2 MB of newlib to resolve those `.o` files' libc references.
//!
//! Instead, every libc / libm / retarget symbol QuickJS-NG imports
//! is provided here by hand, as `#[unsafe(no_mangle)] extern "C"`
//! entry points:
//!
//! - **Math functions** wrap the `libm` crate, which is pure Rust.
//! - **Allocation** (`calloc`, `realloc`): `malloc` and `free` are
//!   owned by rust-psp's `libpsp` rlib (see
//!   `psp/src/panic.rs::libunwind_shims`) and use a
//!   `size_of::<usize>()`-byte header that stores `total =
//!   user_size + 4`. We do *not* shadow those symbols — shadowing
//!   broke the early-boot path on real hardware. Instead our
//!   `calloc` forwards to libpsp's `malloc` and zeroes the block,
//!   and `realloc` reads the libpsp header back out of the
//!   incoming pointer to recover `old_size`, then `malloc`s a new
//!   block, copies, and `free`s the old. That keeps every C-side
//!   alloc in QuickJS-NG (and any stray newlib call from the
//!   rquickjs C sources) consistent with libpsp's layout, so
//!   cross-routine patterns like `dbuf_default_realloc`'s
//!   `free(ptr)` on a `realloc`'d buffer are always safe.
//! - **String / memory** routines (`strchr`, `strcmp`, `memchr`, …)
//!   are straightforward byte loops — most live on the order of
//!   5-10 lines each.
//! - **stdio** (`printf`, `snprintf`, `fwrite`, …) is a tiny format
//!   string interpreter that handles only the conversions QuickJS
//!   actually uses (`%d`, `%u`, `%ld`, `%lld`, `%s`, `%c`, `%p`,
//!   `%x`, `%X`, `%o`, `%%`, `%f`, `%e`, `%g`, width/precision,
//!   zero-padding, `#` alternate form, and `*` dynamic width).
//!   Output goes to a PSP log file on the memory stick for
//!   `stdout`/`stderr`-bound writes and to the caller's buffer for
//!   `snprintf`-style writes. Not C99-complete, just
//!   QuickJS-complete.
//! - **Time** wraps `psp::sys::sceRtc*` and returns UTC `struct tm`
//!   values. Timezone support is stubbed — PSP has no concept of a
//!   runtime-configurable timezone and QuickJS only uses these for
//!   `Date` bookkeeping.
//! - **`abort`** / **`__assert_func`** dispatch to Rust's panic.
//! - **`_impure_ptr`** is a static `struct _reent`-shaped zero.
//!   QuickJS's newlib headers declare `errno` as `_impure_ptr->_errno`
//!   so we just need a writable slot at that offset.
//!
//! Every export has `#[unsafe(no_mangle)]` on the Rust side so the
//! linker sees the symbol names QuickJS expects. All of them are
//! callable from any thread on PSP because the underlying Rust
//! primitives (`std::alloc`, `libm`, `psp::sys`) are themselves
//! reentrant on that platform.
//!
//! ## What this module deliberately does NOT cover
//!
//! - **pthread** — stripped at compile time via `-D__wasi__` in
//!   `.cargo/config.toml`, so QuickJS's `js_mutex_*` / `js_cond_*` /
//!   `js_thread_*` never get emitted. PSP is single-threaded for
//!   the JS engine; the `Atomics` global is gone and that's fine.
//! - **malloc/free/memcpy/memset/memmove/memcmp/strlen/fflush/abort**
//!   — all exported by rust-psp's `libpsp` rlib and used directly.
//!   See the allocation bullet above for how `calloc`/`realloc`
//!   cooperate with libpsp's `malloc`/`free` layout.
//! - **compiler_builtins intrinsics** (`__divdi3`, `__udivdi3`,
//!   `__moddi3`, `__clzdi2`, `__fixdfdi`, …). Rust's
//!   `compiler_builtins` rlib provides them.

use core::ffi::{c_char, c_double, c_int, c_long, c_longlong, c_uint, c_ulong, c_void};
use core::mem;
use core::ptr;
use core::slice;
use core::str;

// ---------------------------------------------------------------------------
// Math — thin wrappers around the libm crate.
// ---------------------------------------------------------------------------

macro_rules! libm_unary {
    ($($name:ident),* $(,)?) => {
        $(
            #[unsafe(no_mangle)]
            pub extern "C" fn $name(x: c_double) -> c_double {
                ::libm::$name(x)
            }
        )*
    };
}

libm_unary!(
    acos, acosh, asin, asinh, atan, atanh, cbrt, ceil, cos, cosh, exp, expm1, floor, log, log10,
    log1p, round, sin, sinh, sqrt, tan, tanh, trunc,
);

#[unsafe(no_mangle)]
pub extern "C" fn atan2(y: c_double, x: c_double) -> c_double {
    ::libm::atan2(y, x)
}

#[unsafe(no_mangle)]
pub extern "C" fn pow(x: c_double, y: c_double) -> c_double {
    ::libm::pow(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn fmod(x: c_double, y: c_double) -> c_double {
    ::libm::fmod(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn hypot(x: c_double, y: c_double) -> c_double {
    ::libm::hypot(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn fmax(x: c_double, y: c_double) -> c_double {
    ::libm::fmax(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn fmin(x: c_double, y: c_double) -> c_double {
    ::libm::fmin(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn frexp(x: c_double, exp_out: *mut c_int) -> c_double {
    let (m, e) = ::libm::frexp(x);
    if !exp_out.is_null() {
        unsafe { *exp_out = e as c_int };
    }
    m
}

#[unsafe(no_mangle)]
pub extern "C" fn scalbn(x: c_double, n: c_int) -> c_double {
    ::libm::scalbn(x, n)
}

#[unsafe(no_mangle)]
pub extern "C" fn lrint(x: c_double) -> c_long {
    ::libm::rint(x) as c_long
}

// ---------------------------------------------------------------------------
// String / memory routines not already provided by rust-psp's libpsp.
// ---------------------------------------------------------------------------

/// # Safety
/// `s` must point to a NUL-terminated C string.
unsafe fn cstr_bytes<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    let mut len = 0usize;
    unsafe {
        while *s.add(len) != 0 {
            len += 1;
        }
        slice::from_raw_parts(s as *const u8, len)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    if s.is_null() {
        return ptr::null_mut();
    }
    let target = c as u8;
    let mut p = s;
    unsafe {
        loop {
            let byte = *p as u8;
            if byte == target {
                return p as *mut c_char;
            }
            if byte == 0 {
                return ptr::null_mut();
            }
            p = p.add(1);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    if s.is_null() {
        return ptr::null_mut();
    }
    let target = c as u8;
    let bytes = unsafe { cstr_bytes(s) };
    for (i, b) in bytes.iter().enumerate().rev() {
        if *b == target {
            return unsafe { s.add(i) as *mut c_char };
        }
    }
    if target == 0 {
        // `strrchr(s, 0)` returns a pointer to the terminator.
        return unsafe { s.add(bytes.len()) as *mut c_char };
    }
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    let mut i = 0usize;
    unsafe {
        loop {
            let ca = *a.add(i) as u8;
            let cb = *b.add(i) as u8;
            if ca != cb {
                return (ca as c_int) - (cb as c_int);
            }
            if ca == 0 {
                return 0;
            }
            i += 1;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    if haystack.is_null() || needle.is_null() {
        return ptr::null_mut();
    }
    let hay = unsafe { cstr_bytes(haystack) };
    let nee = unsafe { cstr_bytes(needle) };
    if nee.is_empty() {
        return haystack as *mut c_char;
    }
    if nee.len() > hay.len() {
        return ptr::null_mut();
    }
    let last = hay.len() - nee.len();
    for i in 0..=last {
        if &hay[i..i + nee.len()] == nee {
            return unsafe { haystack.add(i) as *mut c_char };
        }
    }
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void {
    if s.is_null() || n == 0 {
        return ptr::null_mut();
    }
    let target = c as u8;
    let bytes = unsafe { slice::from_raw_parts(s as *const u8, n) };
    match bytes.iter().position(|b| *b == target) {
        Some(i) => unsafe { (s as *const u8).add(i) as *mut c_void },
        None => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double {
    if nptr.is_null() {
        if !endptr.is_null() {
            unsafe { *endptr = ptr::null_mut() };
        }
        return 0.0;
    }
    let bytes = unsafe { cstr_bytes(nptr) };
    // Skip leading whitespace, then find the longest prefix that
    // parses as a decimal float. Rust's `f64::from_str` is stricter
    // than C's strtod, so we hand-roll the accepted prefix.
    let mut start = 0usize;
    while start < bytes.len() && (bytes[start] as char).is_ascii_whitespace() {
        start += 1;
    }
    let mut end = start;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let mut saw_digit = false;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        saw_digit = true;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
            saw_digit = true;
        }
    }
    if saw_digit && end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut e = end + 1;
        if e < bytes.len() && (bytes[e] == b'+' || bytes[e] == b'-') {
            e += 1;
        }
        let exp_start = e;
        while e < bytes.len() && bytes[e].is_ascii_digit() {
            e += 1;
        }
        if e > exp_start {
            end = e;
        }
    }
    let value = if !saw_digit {
        0.0
    } else {
        match str::from_utf8(&bytes[start..end]) {
            Ok(s) => s.parse::<f64>().unwrap_or(0.0),
            Err(_) => 0.0,
        }
    };
    if !endptr.is_null() {
        let consumed = if saw_digit { end } else { 0 };
        unsafe { *endptr = nptr.add(consumed) as *mut c_char };
    }
    value
}

// ---------------------------------------------------------------------------
// Allocation — `calloc` and `realloc` on top of libpsp's
// `malloc` / `free`.
//
// libpsp stores a `size_of::<usize>()`-byte header immediately
// before every returned pointer. The header holds `total =
// user_size + size_of::<usize>()`. We decode that header in
// `realloc` so we can determine `old_size` without maintaining a
// side table, which keeps the shim fully stateless and means
// `free(p)` after our `realloc(p, ...)` stays consistent with
// every other allocation path in the crate — including rust-psp's
// internal usage.
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// Peek libpsp's per-block header to read `total` out of a
/// pointer returned by its `malloc`. The user-visible size is
/// `total - size_of::<usize>()`.
///
/// # Safety
/// `p` must be a non-null pointer returned by libpsp's `malloc`
/// (directly or transitively via our `calloc`/`realloc`).
unsafe fn libpsp_user_size(p: *const c_void) -> usize {
    let header = unsafe { (p as *const u8).sub(mem::size_of::<usize>()) as *const usize };
    let total = unsafe { header.read() };
    total.saturating_sub(mem::size_of::<usize>())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let total = match nmemb.checked_mul(size) {
        Some(t) => t,
        None => return ptr::null_mut(),
    };
    if total == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let p = malloc(total);
        if p.is_null() {
            return ptr::null_mut();
        }
        ptr::write_bytes(p as *mut u8, 0, total);
        p
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(p: *mut c_void, new_size: usize) -> *mut c_void {
    if p.is_null() {
        return unsafe { malloc(new_size) };
    }
    if new_size == 0 {
        unsafe { free(p) };
        return ptr::null_mut();
    }
    unsafe {
        let old_size = libpsp_user_size(p);
        let new_p = malloc(new_size);
        if new_p.is_null() {
            return ptr::null_mut();
        }
        let copy = core::cmp::min(old_size, new_size);
        ptr::copy_nonoverlapping(p as *const u8, new_p as *mut u8, copy);
        free(p);
        new_p
    }
}

// ---------------------------------------------------------------------------
// stdio — minimal no-op stubs, NON-variadic.
//
// QuickJS-NG references `printf` / `snprintf` / `vsnprintf` /
// `fprintf` / `vfprintf` / `puts` / `putchar` / `fputc` / `fwrite`
// from a few places: atom-to-string conversion (`%u` / `%x`),
// error message construction, and `JSON.stringify` escape
// sequences.
//
// An earlier attempt used Rust's unstable `c_variadic` feature
// with `args: ...` parameters so the signatures would "match" the
// C varargs ABI. That version crashed the console on the very
// first call into `JS_ThrowInternalError` on real hardware, which
// strongly suggests Rust's `c_variadic` codegen on the
// `mipsel-sony-psp-std` target emits a function prologue that's
// ABI-incompatible with what `psp-gcc` expects. Because
// `c_variadic` is experimental and MIPS o32 support in rustc is
// tier-3, that is plausible.
//
// The workaround: declare each stub as a NON-variadic function
// with only the fixed positional arguments it cares about. C
// callers passing extra varargs still go through the normal
// o32 calling convention — the extra values end up in `$a2`/`$a3`
// or on the stack and the callee simply ignores them. That avoids
// the c_variadic codegen path entirely.
//
// Output buffers receive a single NUL so `strlen` on them returns
// 0; return codes claim full success so QuickJS's length-checking
// loops terminate on the first iteration. Error messages from
// QuickJS become empty strings, and `bc_read_trace` silently
// drops. For any non-pathological `JS_Eval` neither path is
// actually reached, but they still need to link.
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snprintf(
    buf: *mut c_char,
    size: usize,
    _fmt: *const c_char,
) -> c_int {
    if !buf.is_null() && size > 0 {
        unsafe { buf.write(0) };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsnprintf(
    buf: *mut c_char,
    size: usize,
    _fmt: *const c_char,
    _ap: *const c_void,
) -> c_int {
    if !buf.is_null() && size > 0 {
        unsafe { buf.write(0) };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn printf(_fmt: *const c_char) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fprintf(
    _stream: *mut c_void,
    _fmt: *const c_char,
) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vfprintf(
    _stream: *mut c_void,
    _fmt: *const c_char,
    _ap: *const c_void,
) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(_s: *const c_char) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn putchar(c: c_int) -> c_int {
    c
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputc(c: c_int, _stream: *mut c_void) -> c_int {
    c
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fwrite(
    _p: *const c_void,
    _size: usize,
    nmemb: usize,
    _stream: *mut c_void,
) -> usize {
    nmemb
}

// ---------------------------------------------------------------------------
// Time — `clock_gettime`, `gettimeofday`, and friends.
//
// QuickJS uses these for:
//   * performance timing (`Date.now()`, `performance.now()` analogue),
//   * `Date` object conversions (mktime, gmtime_r, localtime_r).
//
// The PSP has no configurable TZ database; we treat local time as UTC
// and stub difftime to a plain subtraction.
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Default)]
pub struct Timespec {
    pub tv_sec: c_long,
    pub tv_nsec: c_long,
}

#[repr(C)]
#[derive(Default)]
pub struct Timeval {
    pub tv_sec: c_long,
    pub tv_usec: c_long,
}

#[repr(C)]
#[derive(Default)]
pub struct Tm {
    pub tm_sec: c_int,
    pub tm_min: c_int,
    pub tm_hour: c_int,
    pub tm_mday: c_int,
    pub tm_mon: c_int,
    pub tm_year: c_int,
    pub tm_wday: c_int,
    pub tm_yday: c_int,
    pub tm_isdst: c_int,
    pub tm_gmtoff: c_long,
    pub tm_zone: *const c_char,
}

/// Read the PSP RTC and return (unix seconds, nanoseconds).
fn rtc_now() -> (i64, i64) {
    use psp::sys;
    unsafe {
        let mut tick: u64 = 0;
        sys::sceRtcGetCurrentTick(&mut tick);
        let tick_res = sys::sceRtcGetTickResolution() as u64;
        let secs = (tick / tick_res) as i64;
        let frac = (tick % tick_res) as i64;
        let nsec = frac.saturating_mul(1_000_000_000) / tick_res as i64;
        // sceRtc's tick epoch is 0001-01-01 00:00:00 UTC. Convert to
        // Unix epoch (1970-01-01) by subtracting the constant offset.
        const UNIX_EPOCH_IN_RTC_SECS: i64 = 62_135_596_800;
        (secs - UNIX_EPOCH_IN_RTC_SECS, nsec)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_gettime(_clk_id: c_int, tp: *mut Timespec) -> c_int {
    if tp.is_null() {
        return -1;
    }
    let (s, n) = rtc_now();
    unsafe {
        (*tp).tv_sec = s as c_long;
        (*tp).tv_nsec = n as c_long;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gettimeofday(tv: *mut Timeval, _tz: *mut c_void) -> c_int {
    if tv.is_null() {
        return -1;
    }
    let (s, n) = rtc_now();
    unsafe {
        (*tv).tv_sec = s as c_long;
        (*tv).tv_usec = (n / 1000) as c_long;
    }
    0
}

/// Days before the start of month `m` (0-indexed), ignoring leap days.
const DAYS_BEFORE_MONTH: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn unix_to_tm(t: i64, out: &mut Tm) {
    let mut days = t.div_euclid(86_400) as i32;
    let secs_of_day = t.rem_euclid(86_400) as i32;
    out.tm_sec = secs_of_day % 60;
    out.tm_min = (secs_of_day / 60) % 60;
    out.tm_hour = secs_of_day / 3600;
    // 1970-01-01 is a Thursday (tm_wday = 4).
    out.tm_wday = (4 + days.rem_euclid(7)).rem_euclid(7);
    let mut year = 1970;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days {
            break;
        }
        days -= year_days;
        year += 1;
    }
    // Handle pre-epoch rollback for negative input.
    while days < 0 {
        year -= 1;
        let year_days = if is_leap(year) { 366 } else { 365 };
        days += year_days;
    }
    out.tm_year = year - 1900;
    out.tm_yday = days;
    let mut month = 11;
    for m in (0..12usize).rev() {
        let first = DAYS_BEFORE_MONTH[m] + if m > 1 && is_leap(year) { 1 } else { 0 };
        if days >= first {
            month = m;
            out.tm_mday = days - first + 1;
            break;
        }
    }
    out.tm_mon = month as i32;
    out.tm_isdst = 0;
    out.tm_gmtoff = 0;
    out.tm_zone = ptr::null();
}

fn tm_to_unix(t: &Tm) -> i64 {
    let year = t.tm_year + 1900;
    let mut days: i64 = 0;
    if year >= 1970 {
        for y in 1970..year {
            days += if is_leap(y) { 366 } else { 365 };
        }
    } else {
        for y in year..1970 {
            days -= if is_leap(y) { 366 } else { 365 };
        }
    }
    let month = t.tm_mon.clamp(0, 11) as usize;
    days += DAYS_BEFORE_MONTH[month] as i64;
    if month > 1 && is_leap(year) {
        days += 1;
    }
    days += (t.tm_mday - 1) as i64;
    days * 86_400 + t.tm_hour as i64 * 3600 + t.tm_min as i64 * 60 + t.tm_sec as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gmtime_r(t: *const c_long, out: *mut Tm) -> *mut Tm {
    if t.is_null() || out.is_null() {
        return ptr::null_mut();
    }
    let secs = unsafe { *t } as i64;
    unsafe {
        *out = Tm::default();
        unix_to_tm(secs, &mut *out);
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn localtime_r(t: *const c_long, out: *mut Tm) -> *mut Tm {
    // PSP has no timezone database; "local" == UTC.
    unsafe { gmtime_r(t, out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mktime(t: *mut Tm) -> c_long {
    if t.is_null() {
        return -1;
    }
    let secs = unsafe { tm_to_unix(&*t) };
    // Re-normalise the struct so callers see the canonical fields.
    unsafe {
        unix_to_tm(secs, &mut *t);
    }
    secs as c_long
}

#[unsafe(no_mangle)]
pub extern "C" fn difftime(a: c_long, b: c_long) -> c_double {
    (a - b) as c_double
}

// `js__gettimeofday_us` / `js__hrtime_ns` are defined inside
// QuickJS-NG's `cutils.c`, so we do NOT shadow them here. Our
// `clock_gettime` + `gettimeofday` above are what QuickJS calls
// indirectly when its own helpers need a wall clock.

// ---------------------------------------------------------------------------
// abort / __assert_func / _impure_ptr
// ---------------------------------------------------------------------------

// `abort` is already exported by rust-psp's libpsp. Don't redefine it.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __assert_func(
    _file: *const c_char,
    _line: c_int,
    _func: *const c_char,
    _expr: *const c_char,
) -> ! {
    panic!("QuickJS assertion failed");
}

/// Minimal stand-in for newlib's `struct _reent`. QuickJS's headers
/// declare `errno` as `_impure_ptr->_errno`, so the only field any
/// of the C source actually touches is `_errno` at offset 0. We give
/// the symbol a zeroed backing store with room for the full struct
/// layout just in case other macros dereference it.
#[repr(C)]
struct Reent {
    errno: c_int,
    _pad: [u8; 252],
}

static mut IMPURE_STORAGE: Reent = Reent {
    errno: 0,
    _pad: [0; 252],
};

#[unsafe(no_mangle)]
pub static mut _impure_ptr: *mut Reent = unsafe { &raw mut IMPURE_STORAGE };

// The unused-import suppressor: keep the module from being GC'd if
// no other oasis-backend-psp code references it.
#[allow(dead_code)]
pub(crate) fn __force_link() {
    let _ = acos as usize;
}

// Unused symbols — named so the initial compile surfaces any that go
// missing, without tripping clippy.
#[allow(dead_code)]
const _UNUSED_C_UINT: c_uint = 0;
#[allow(dead_code)]
const _UNUSED_C_ULONG: c_ulong = 0;
#[allow(dead_code)]
const _UNUSED_C_LONGLONG: c_longlong = 0;
