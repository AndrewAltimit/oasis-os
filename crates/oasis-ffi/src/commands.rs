//! Command dispatch: `oasis_send_command` and `oasis_free_string`.

use std::ffi::CString;
use std::os::raw::c_char;

use oasis_core::terminal::{CommandOutput, CommandSignal, Environment};

use crate::handle::{OasisInstance, c_str_to_str, ffi_guard, with_instance};
use crate::types::OASIS_CB_COMMAND_EXEC;

/// Execute a terminal command and return the output as a C string.
///
/// The caller must free the returned string with `oasis_free_string`.
/// Returns null for a null handle, a null / non-UTF-8 `cmd`, or a caught
/// panic; a failing command returns its `error: ...` text instead.
///
/// # Safety
///
/// `handle` must be null or valid. `cmd` must be null or a valid
/// null-terminated C string.
///
/// # Thread Safety
///
/// Caller must ensure single-threaded access to the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_send_command(
    handle: *mut OasisInstance,
    cmd: *const c_char,
) -> *mut c_char {
    ffi_guard("oasis_send_command", std::ptr::null_mut(), || {
        // SAFETY: Caller guarantees pointer is null or a valid C string per function safety contract.
        let Some(cmd_str) = (unsafe { c_str_to_str(cmd) }) else {
            return std::ptr::null_mut();
        };

        // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
        unsafe {
            with_instance(handle, std::ptr::null_mut(), |instance| {
                send_command_inner(instance, cmd_str)
            })
        }
    })
}

fn send_command_inner(instance: &mut OasisInstance, cmd_str: &str) -> *mut c_char {
    #[cfg(test)]
    crate::handle::maybe_inject_panic();

    instance.fire_callback(OASIS_CB_COMMAND_EXEC, cmd_str);

    let mut env = Environment {
        cwd: instance.cwd.clone(),
        vfs: &mut instance.vfs,
        power: Some(&instance.platform),
        time: Some(&instance.platform),
        usb: Some(&instance.platform),

        network: None,
        tls: None,
        stdin: None,
        stderr: String::new(),
    };

    let output = match instance.cmd_reg.execute(cmd_str, &mut env) {
        Ok(output) => output_text(output),
        Err(e) => format!("error: {e}"),
    };

    instance.cwd = env.cwd;

    output_to_c_string(output).into_raw()
}

/// Flatten a command's output to the text a C host sees: tables as
/// `|`-separated rows, `Multi` parts joined by newlines, shell signals the
/// embedded instance can't act on as a short notice.
fn output_text(output: CommandOutput) -> String {
    match output {
        CommandOutput::Text(text) => text,
        CommandOutput::Table { headers, rows } => {
            let mut out = headers.join(" | ");
            for row in &rows {
                out.push('\n');
                out.push_str(&row.join(" | "));
            }
            out
        },
        // `Status` is unwrapped by the executor and never reaches hosts.
        CommandOutput::Clear | CommandOutput::None | CommandOutput::Status { .. } => String::new(),
        CommandOutput::Signal(sig) => match sig {
            CommandSignal::BrowserSandbox { enable } => {
                let state = if enable { "on" } else { "off" };
                format!("Browser sandbox: {state}")
            },
            CommandSignal::SkinSwap { name } => {
                format!("Skin swap to '{name}' not available via FFI.")
            },
            _ => "Not available via FFI.".to_string(),
        },
        CommandOutput::Multi(outputs) => outputs
            .into_iter()
            .map(output_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
pub(crate) fn output_text_for_test(output: CommandOutput) -> String {
    output_text(output)
}

/// Convert command output to a C string. Interior NUL bytes (e.g. `cat` of a
/// binary file) cannot be represented, so each is replaced with U+FFFD
/// instead of failing the whole call -- NULL is reserved for invalid input.
fn output_to_c_string(output: String) -> CString {
    CString::new(output).unwrap_or_else(|e| {
        let text = String::from_utf8_lossy(&e.into_vec()).replace('\0', "\u{FFFD}");
        CString::new(text).unwrap_or_default()
    })
}

/// Free a string previously returned by `oasis_send_command`.
///
/// # Safety
///
/// `ptr` must be a pointer returned by `oasis_send_command`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_free_string(ptr: *mut c_char) {
    ffi_guard("oasis_free_string", (), || {
        if !ptr.is_null() {
            // SAFETY: Reclaiming ownership of CString allocated by `CString::into_raw`.
            drop(unsafe { CString::from_raw(ptr) });
        }
    })
}
