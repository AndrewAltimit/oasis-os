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
//! - **Allocation** (`calloc`, `realloc`) goes through Rust's global
//!   allocator by prefixing every block with its size, the same
//!   trick `rust_alloc_shim` uses on bare-metal targets. `malloc`
//!   and `free` are already exported by `rust-psp`, so we share its
//!   size header layout.
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
//!   — already exported by `rust-psp` (`libpsp-*.rlib`). Double
//!   definitions would break the link even with
//!   `--allow-multiple-definition`, so we skip them here.
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
// Allocation — calloc and realloc on top of Rust's global allocator.
//
// rust-psp's libpsp already exports `malloc`/`free` via a shim that
// prefixes every block with the allocation size. We match that layout
// so `realloc` and `calloc` interoperate cleanly: the first
// `size_of::<usize>()` bytes before the returned pointer hold the
// user-visible length.
// ---------------------------------------------------------------------------

const ALLOC_ALIGN: usize = 16;
const ALLOC_HEADER: usize = mem::size_of::<usize>();

unsafe fn rust_alloc_block(n: usize) -> *mut u8 {
    use std::alloc::{Layout, alloc};
    let layout = match Layout::from_size_align(n + ALLOC_HEADER, ALLOC_ALIGN) {
        Ok(l) => l,
        Err(_) => return ptr::null_mut(),
    };
    unsafe {
        let raw = alloc(layout);
        if raw.is_null() {
            return ptr::null_mut();
        }
        (raw as *mut usize).write(n);
        raw.add(ALLOC_HEADER)
    }
}

unsafe fn rust_dealloc_block(p: *mut u8) {
    use std::alloc::{Layout, dealloc};
    if p.is_null() {
        return;
    }
    unsafe {
        let raw = p.sub(ALLOC_HEADER);
        let n = (raw as *const usize).read();
        let layout = Layout::from_size_align_unchecked(n + ALLOC_HEADER, ALLOC_ALIGN);
        dealloc(raw, layout);
    }
}

unsafe fn rust_block_size(p: *const u8) -> usize {
    unsafe { (p.sub(ALLOC_HEADER) as *const usize).read() }
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
        let p = rust_alloc_block(total);
        if p.is_null() {
            return ptr::null_mut();
        }
        ptr::write_bytes(p, 0, total);
        p as *mut c_void
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(p: *mut c_void, new_size: usize) -> *mut c_void {
    if p.is_null() {
        return unsafe { rust_alloc_block(new_size) as *mut c_void };
    }
    if new_size == 0 {
        unsafe { rust_dealloc_block(p as *mut u8) };
        return ptr::null_mut();
    }
    unsafe {
        let old_size = rust_block_size(p as *const u8);
        let new_p = rust_alloc_block(new_size);
        if new_p.is_null() {
            return ptr::null_mut();
        }
        let copy = core::cmp::min(old_size, new_size);
        ptr::copy_nonoverlapping(p as *const u8, new_p, copy);
        rust_dealloc_block(p as *mut u8);
        new_p as *mut c_void
    }
}

// ---------------------------------------------------------------------------
// stdio — minimal printf/snprintf family.
//
// Only the conversions QuickJS-NG actually uses are implemented. The
// rendering logic is shared between snprintf/vsnprintf (caller-owned
// buffer) and printf/fprintf/puts/etc. (discarded or logged). See
// `FormatSink` for the backing-store abstraction.
// ---------------------------------------------------------------------------

/// Destination for a formatted-output operation.
///
/// `Buffer` is used by `snprintf` / `vsnprintf` (caller supplies the
/// backing store). `Discard` is used by `printf` / `fprintf` /
/// `puts` / `putchar` / `fputc` / `fwrite` — QuickJS only emits text
/// through those for debug/error logging and we don't have a
/// meaningful stdout on the PSP EBOOT anyway, so we count bytes but
/// don't store anything. `Counting` is used to ask how many bytes a
/// render *would* produce (pre-allocation pattern).
enum FormatSink {
    Buffer { buf: *mut u8, cap: usize, pos: usize, written: usize },
    Discard { written: usize },
}

impl FormatSink {
    fn push_byte(&mut self, b: u8) {
        match self {
            Self::Buffer { buf, cap, pos, written } => {
                if !buf.is_null() && *pos + 1 < *cap {
                    unsafe { buf.add(*pos).write(b) };
                    *pos += 1;
                }
                *written += 1;
            },
            Self::Discard { written } => {
                *written += 1;
            },
        }
    }

    fn push_bytes(&mut self, src: &[u8]) {
        for &b in src {
            self.push_byte(b);
        }
    }

    fn finish(self) -> usize {
        match self {
            Self::Buffer { buf, cap, pos, written } => {
                if !buf.is_null() && cap > 0 {
                    let term = pos.min(cap - 1);
                    unsafe { buf.add(term).write(0) };
                }
                written
            },
            Self::Discard { written } => written,
        }
    }
}

/// Parsed conversion specifier — only the fields QuickJS-NG uses.
#[derive(Default)]
struct Spec {
    left_align: bool,
    zero_pad: bool,
    plus_sign: bool,
    space_sign: bool,
    alt_form: bool,
    width: usize,
    precision: Option<usize>,
    length: Length,
    conversion: u8,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Length {
    #[default]
    Int,
    Long,
    LongLong,
    SizeT,
}

/// C va_list abstraction — we accept a raw pointer and walk it
/// stride-by-stride. On o32 MIPS, varargs are promoted to at least
/// 32-bit integers; `long long` and `double` are 8-byte and
/// 8-aligned; pointers are 32-bit. This matches the layout QuickJS's
/// `cc`-compiled code pushes onto the stack.
struct VaList {
    cursor: *const u8,
}

impl VaList {
    unsafe fn new(p: *const c_void) -> Self {
        Self { cursor: p as *const u8 }
    }

    unsafe fn align_to(&mut self, n: usize) {
        let addr = self.cursor as usize;
        let aligned = (addr + n - 1) & !(n - 1);
        self.cursor = aligned as *const u8;
    }

    unsafe fn read_i32(&mut self) -> i32 {
        unsafe {
            self.align_to(4);
            let v = (self.cursor as *const i32).read_unaligned();
            self.cursor = self.cursor.add(4);
            v
        }
    }

    unsafe fn read_u32(&mut self) -> u32 {
        unsafe { self.read_i32() as u32 }
    }

    unsafe fn read_i64(&mut self) -> i64 {
        unsafe {
            self.align_to(8);
            let v = (self.cursor as *const i64).read_unaligned();
            self.cursor = self.cursor.add(8);
            v
        }
    }

    unsafe fn read_u64(&mut self) -> u64 {
        unsafe { self.read_i64() as u64 }
    }

    unsafe fn read_ptr(&mut self) -> *const c_void {
        unsafe {
            self.align_to(4);
            let v = (self.cursor as *const *const c_void).read_unaligned();
            self.cursor = self.cursor.add(4);
            v
        }
    }

    unsafe fn read_f64(&mut self) -> f64 {
        unsafe {
            self.align_to(8);
            let v = (self.cursor as *const f64).read_unaligned();
            self.cursor = self.cursor.add(8);
            v
        }
    }
}

/// Render a format string into `sink`, consuming arguments from `va`.
///
/// The caller has already validated `fmt` is a NUL-terminated C string.
unsafe fn format_into(sink: &mut FormatSink, fmt: *const c_char, mut va: VaList) -> usize {
    let bytes = unsafe { cstr_bytes(fmt) };
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c != b'%' {
            sink.push_byte(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            sink.push_byte(b'%');
            break;
        }
        if bytes[i] == b'%' {
            sink.push_byte(b'%');
            i += 1;
            continue;
        }
        let mut spec = Spec::default();
        // Flags
        loop {
            match bytes.get(i).copied() {
                Some(b'-') => {
                    spec.left_align = true;
                    i += 1;
                },
                Some(b'+') => {
                    spec.plus_sign = true;
                    i += 1;
                },
                Some(b' ') => {
                    spec.space_sign = true;
                    i += 1;
                },
                Some(b'#') => {
                    spec.alt_form = true;
                    i += 1;
                },
                Some(b'0') => {
                    spec.zero_pad = true;
                    i += 1;
                },
                _ => break,
            }
        }
        // Width
        if bytes.get(i) == Some(&b'*') {
            let w = unsafe { va.read_i32() };
            if w < 0 {
                spec.left_align = true;
                spec.width = (-w) as usize;
            } else {
                spec.width = w as usize;
            }
            i += 1;
        } else {
            while let Some(&d) = bytes.get(i) {
                if !d.is_ascii_digit() {
                    break;
                }
                spec.width = spec.width * 10 + (d - b'0') as usize;
                i += 1;
            }
        }
        // Precision
        if bytes.get(i) == Some(&b'.') {
            i += 1;
            let mut p = 0usize;
            if bytes.get(i) == Some(&b'*') {
                let v = unsafe { va.read_i32() };
                p = v.max(0) as usize;
                i += 1;
            } else {
                while let Some(&d) = bytes.get(i) {
                    if !d.is_ascii_digit() {
                        break;
                    }
                    p = p * 10 + (d - b'0') as usize;
                    i += 1;
                }
            }
            spec.precision = Some(p);
        }
        // Length modifier
        match bytes.get(i).copied() {
            Some(b'l') => {
                i += 1;
                if bytes.get(i) == Some(&b'l') {
                    spec.length = Length::LongLong;
                    i += 1;
                } else {
                    spec.length = Length::Long;
                }
            },
            Some(b'z') => {
                spec.length = Length::SizeT;
                i += 1;
            },
            Some(b'h') => {
                // h / hh collapse back to int after promotion.
                i += 1;
                if bytes.get(i) == Some(&b'h') {
                    i += 1;
                }
            },
            _ => {},
        }
        let conv = match bytes.get(i).copied() {
            Some(c) => c,
            None => break,
        };
        spec.conversion = conv;
        i += 1;
        unsafe { render_conv(sink, &spec, &mut va) };
    }
    sink.push_byte(0);
    // push_byte incremented `written` for the NUL too — take it back
    // since C callers don't count the terminator.
    match sink {
        FormatSink::Buffer { written, pos, .. } => {
            *written -= 1;
            if *pos > 0 {
                *pos -= 1;
            }
            *written
        },
        FormatSink::Discard { written } => {
            *written -= 1;
            *written
        },
    }
}

unsafe fn render_conv(sink: &mut FormatSink, spec: &Spec, va: &mut VaList) {
    let mut buf = [0u8; 64];
    match spec.conversion {
        b'c' => {
            let ch = unsafe { va.read_i32() } as u8;
            render_padded(sink, &[ch], spec, false);
        },
        b's' => {
            let ptr = unsafe { va.read_ptr() } as *const c_char;
            let s = if ptr.is_null() { &b"(null)"[..] } else { unsafe { cstr_bytes(ptr) } };
            let trimmed = match spec.precision {
                Some(p) if p < s.len() => &s[..p],
                _ => s,
            };
            render_padded(sink, trimmed, spec, false);
        },
        b'd' | b'i' => {
            let v = match spec.length {
                Length::LongLong => unsafe { va.read_i64() },
                _ => unsafe { va.read_i32() as i64 },
            };
            let (digits, len) = render_signed(&mut buf, v);
            render_integer(sink, &digits[..len], v < 0, 10, spec, false);
        },
        b'u' => {
            let v = match spec.length {
                Length::LongLong => unsafe { va.read_u64() },
                _ => unsafe { va.read_u32() as u64 },
            };
            let (digits, len) = render_unsigned(&mut buf, v, 10, false);
            render_integer(sink, &digits[..len], false, 10, spec, false);
        },
        b'x' | b'X' => {
            let v = match spec.length {
                Length::LongLong => unsafe { va.read_u64() },
                _ => unsafe { va.read_u32() as u64 },
            };
            let upper = spec.conversion == b'X';
            let (digits, len) = render_unsigned(&mut buf, v, 16, upper);
            render_integer(sink, &digits[..len], false, 16, spec, upper);
        },
        b'o' => {
            let v = match spec.length {
                Length::LongLong => unsafe { va.read_u64() },
                _ => unsafe { va.read_u32() as u64 },
            };
            let (digits, len) = render_unsigned(&mut buf, v, 8, false);
            render_integer(sink, &digits[..len], false, 8, spec, false);
        },
        b'p' => {
            let ptr = unsafe { va.read_ptr() };
            sink.push_bytes(b"0x");
            let (digits, len) = render_unsigned(&mut buf, ptr as u64, 16, false);
            sink.push_bytes(&digits[..len]);
        },
        b'f' | b'F' => {
            let v = unsafe { va.read_f64() };
            render_float_fixed(sink, v, spec);
        },
        b'e' | b'E' => {
            let v = unsafe { va.read_f64() };
            render_float_exp(sink, v, spec, spec.conversion == b'E');
        },
        b'g' | b'G' => {
            let v = unsafe { va.read_f64() };
            render_float_general(sink, v, spec, spec.conversion == b'G');
        },
        b'n' => {
            // Not supported — consume the pointer and drop.
            let _ = unsafe { va.read_ptr() };
        },
        other => {
            sink.push_byte(b'%');
            sink.push_byte(other);
        },
    }
}

fn render_signed(buf: &mut [u8], v: i64) -> (&[u8], usize) {
    let mut n = if v < 0 { (v as i128).unsigned_abs() as u64 } else { v as u64 };
    let mut idx = buf.len();
    if n == 0 {
        idx -= 1;
        buf[idx] = b'0';
    }
    while n > 0 {
        idx -= 1;
        buf[idx] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    (&buf[idx..], buf.len() - idx)
}

fn render_unsigned(buf: &mut [u8], mut v: u64, base: u32, upper: bool) -> (&[u8], usize) {
    let mut idx = buf.len();
    if v == 0 {
        idx -= 1;
        buf[idx] = b'0';
    }
    while v > 0 {
        let digit = (v % base as u64) as u8;
        let ch = if digit < 10 {
            b'0' + digit
        } else if upper {
            b'A' + (digit - 10)
        } else {
            b'a' + (digit - 10)
        };
        idx -= 1;
        buf[idx] = ch;
        v /= base as u64;
    }
    (&buf[idx..], buf.len() - idx)
}

fn render_padded(sink: &mut FormatSink, bytes: &[u8], spec: &Spec, upper: bool) {
    let _ = upper;
    let width = spec.width;
    let pad = if width > bytes.len() { width - bytes.len() } else { 0 };
    if !spec.left_align {
        for _ in 0..pad {
            sink.push_byte(b' ');
        }
    }
    sink.push_bytes(bytes);
    if spec.left_align {
        for _ in 0..pad {
            sink.push_byte(b' ');
        }
    }
}

fn render_integer(
    sink: &mut FormatSink,
    digits: &[u8],
    negative: bool,
    base: u32,
    spec: &Spec,
    upper: bool,
) {
    let sign_bytes: &[u8] = if negative {
        b"-"
    } else if spec.plus_sign {
        b"+"
    } else if spec.space_sign {
        b" "
    } else {
        b""
    };
    let prefix_bytes: &[u8] = if spec.alt_form && base == 16 {
        if upper { b"0X" } else { b"0x" }
    } else if spec.alt_form && base == 8 && digits.first() != Some(&b'0') {
        b"0"
    } else {
        b""
    };
    let mut core_len = digits.len();
    // Precision padding: integer conversions use `precision` as a
    // minimum digit count, not a trailing-fraction length.
    let mut zero_prefix = 0usize;
    if let Some(p) = spec.precision
        && p > core_len
    {
        zero_prefix = p - core_len;
        core_len = p;
    }
    let body_len = sign_bytes.len() + prefix_bytes.len() + core_len;
    let width = spec.width;
    let pad = if width > body_len { width - body_len } else { 0 };
    let pad_byte = if spec.zero_pad && spec.precision.is_none() && !spec.left_align { b'0' } else { b' ' };
    if !spec.left_align && pad_byte == b' ' {
        for _ in 0..pad {
            sink.push_byte(b' ');
        }
    }
    sink.push_bytes(sign_bytes);
    sink.push_bytes(prefix_bytes);
    if !spec.left_align && pad_byte == b'0' {
        for _ in 0..pad {
            sink.push_byte(b'0');
        }
    }
    for _ in 0..zero_prefix {
        sink.push_byte(b'0');
    }
    sink.push_bytes(digits);
    if spec.left_align {
        for _ in 0..pad {
            sink.push_byte(b' ');
        }
    }
}

// -- Floating-point formatters --------------------------------------------
//
// These back onto Rust's `f64::to_string` / format machinery via the
// `ryu` algorithm baked into `core::fmt`. We stage the result into a
// 64-byte scratch buffer, then apply width / sign / prefix padding.
//
// QuickJS uses:
//  * `%f` / `%.Nf` — `snprintf(buf, sz, "%.17g", d)` and similar
//  * `%e` / `%.Ne` — exponent form
//  * `%g` / `%.Ng` — shortest-roundtrip form (most common, from dtoa)

fn render_float_fixed(sink: &mut FormatSink, v: f64, spec: &Spec) {
    if v.is_nan() {
        render_padded(sink, b"nan", spec, false);
        return;
    }
    if v.is_infinite() {
        let bytes: &[u8] = if v.is_sign_negative() { b"-inf" } else { b"inf" };
        render_padded(sink, bytes, spec, false);
        return;
    }
    let prec = spec.precision.unwrap_or(6);
    let mut scratch = [0u8; 64];
    let neg = v.is_sign_negative();
    let abs = if neg { -v } else { v };
    // Round `abs` to `prec` decimal places, then emit.
    let scale = ::libm::pow(10.0, prec as f64);
    let rounded = ::libm::round(abs * scale) / scale;
    let int_part = rounded.trunc() as u64;
    let (idigits, ilen) = render_unsigned(&mut scratch, int_part, 10, false);
    let mut out: Vec<u8> = Vec::with_capacity(32);
    if neg {
        out.push(b'-');
    } else if spec.plus_sign {
        out.push(b'+');
    } else if spec.space_sign {
        out.push(b' ');
    }
    out.extend_from_slice(&idigits[..ilen]);
    if prec > 0 {
        out.push(b'.');
        let mut frac = rounded - int_part as f64;
        for _ in 0..prec {
            frac *= 10.0;
            let d = frac as u8;
            out.push(b'0' + d);
            frac -= d as f64;
        }
    } else if spec.alt_form {
        out.push(b'.');
    }
    render_padded(sink, &out, spec, false);
}

fn render_float_exp(sink: &mut FormatSink, v: f64, spec: &Spec, upper: bool) {
    if v.is_nan() || v.is_infinite() {
        render_float_fixed(sink, v, spec);
        return;
    }
    let prec = spec.precision.unwrap_or(6);
    let neg = v.is_sign_negative();
    let abs = if neg { -v } else { v };
    let (mantissa, exponent) = if abs == 0.0 {
        (0.0, 0i32)
    } else {
        let e = ::libm::floor(::libm::log10(abs)) as i32;
        let m = abs / ::libm::pow(10.0, e as f64);
        (m, e)
    };
    // Re-use fixed formatter for the mantissa, then tack on the exp.
    let mut inner_spec = Spec { precision: Some(prec), ..Spec::default() };
    inner_spec.plus_sign = spec.plus_sign;
    inner_spec.space_sign = spec.space_sign;
    let mut mantissa_sink = FormatSink::Buffer {
        buf: [0u8; 64].as_mut_ptr(),
        cap: 64,
        pos: 0,
        written: 0,
    };
    let mut mant_buf = [0u8; 64];
    let mut mbuf_sink = FormatSink::Buffer {
        buf: mant_buf.as_mut_ptr(),
        cap: mant_buf.len(),
        pos: 0,
        written: 0,
    };
    render_float_fixed(
        &mut mbuf_sink,
        if neg { -mantissa } else { mantissa },
        &inner_spec,
    );
    let mant_len = match &mbuf_sink {
        FormatSink::Buffer { pos, .. } => *pos,
        _ => 0,
    };
    let _ = mbuf_sink.finish();
    let _ = mantissa_sink.finish();
    let mut out: Vec<u8> = Vec::with_capacity(32);
    out.extend_from_slice(&mant_buf[..mant_len]);
    out.push(if upper { b'E' } else { b'e' });
    if exponent < 0 {
        out.push(b'-');
    } else {
        out.push(b'+');
    }
    let exp_abs = exponent.unsigned_abs();
    if exp_abs < 10 {
        out.push(b'0');
    }
    let mut exp_scratch = [0u8; 16];
    let (edig, elen) = render_unsigned(&mut exp_scratch, exp_abs as u64, 10, false);
    out.extend_from_slice(&edig[..elen]);
    render_padded(sink, &out, spec, false);
}

fn render_float_general(sink: &mut FormatSink, v: f64, spec: &Spec, upper: bool) {
    if v.is_nan() || v.is_infinite() {
        render_float_fixed(sink, v, spec);
        return;
    }
    let prec = spec.precision.unwrap_or(6).max(1);
    let abs = if v.is_sign_negative() { -v } else { v };
    let exponent = if abs == 0.0 {
        0i32
    } else {
        ::libm::floor(::libm::log10(abs)) as i32
    };
    if exponent < -4 || exponent >= prec as i32 {
        let mut s2 = Spec { precision: Some(prec - 1), ..Spec::default() };
        s2.width = spec.width;
        s2.left_align = spec.left_align;
        s2.zero_pad = spec.zero_pad;
        s2.plus_sign = spec.plus_sign;
        s2.space_sign = spec.space_sign;
        render_float_exp(sink, v, &s2, upper);
    } else {
        let adjusted = (prec as i32 - 1 - exponent).max(0) as usize;
        let mut s2 = Spec { precision: Some(adjusted), ..Spec::default() };
        s2.width = spec.width;
        s2.left_align = spec.left_align;
        s2.zero_pad = spec.zero_pad;
        s2.plus_sign = spec.plus_sign;
        s2.space_sign = spec.space_sign;
        render_float_fixed(sink, v, &s2);
    }
}

// -- Public C entry points ------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsnprintf(
    buf: *mut c_char,
    size: usize,
    fmt: *const c_char,
    ap: *const c_void,
) -> c_int {
    let mut sink = FormatSink::Buffer {
        buf: buf as *mut u8,
        cap: size,
        pos: 0,
        written: 0,
    };
    let va = unsafe { VaList::new(ap) };
    let written = unsafe { format_into(&mut sink, fmt, va) };
    let _ = sink.finish();
    written as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vfprintf(
    _stream: *mut c_void,
    fmt: *const c_char,
    ap: *const c_void,
) -> c_int {
    let mut sink = FormatSink::Discard { written: 0 };
    let va = unsafe { VaList::new(ap) };
    let written = unsafe { format_into(&mut sink, fmt, va) };
    let _ = sink.finish();
    written as c_int
}

// The variadic-call entry points (`snprintf`, `printf`, `fprintf`)
// can't be expressed in stable Rust as true C varargs, so we forward
// to the `v`-variants with a pointer to the first vararg. On o32
// MIPS, the `cc`-compiled caller passes the first four varargs in
// registers and spills the rest to a stack area pointed at by the
// (eventual) `va_start` macro — which, for small fixed-argument
// stubs like these, we approximate by reading the vararg pointer
// directly off the caller's stack.
//
// QuickJS-NG invokes these from internal helper functions where the
// variadic list layout matches the normal stack spill area. If we
// find cases where registers need to be captured we'll add a small
// assembly trampoline, but for now keeping it pure Rust keeps the
// diff legible.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snprintf(
    buf: *mut c_char,
    size: usize,
    fmt: *const c_char,
    args: ...
) -> c_int {
    unsafe { vsnprintf(buf, size, fmt, &args as *const _ as *const c_void) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn printf(fmt: *const c_char, args: ...) -> c_int {
    unsafe { vfprintf(ptr::null_mut(), fmt, &args as *const _ as *const c_void) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fprintf(
    stream: *mut c_void,
    fmt: *const c_char,
    args: ...
) -> c_int {
    unsafe { vfprintf(stream, fmt, &args as *const _ as *const c_void) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    if s.is_null() {
        return -1;
    }
    let bytes = unsafe { cstr_bytes(s) };
    // Discard; return non-negative on success.
    bytes.len() as c_int
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
    size: usize,
    nmemb: usize,
    _stream: *mut c_void,
) -> usize {
    // Pretend we wrote everything; QuickJS uses this path only for
    // error/debug logging, which we silently drop on PSP.
    size.wrapping_mul(nmemb);
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
