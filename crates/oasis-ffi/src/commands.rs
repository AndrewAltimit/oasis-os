//! Command dispatch: `oasis_send_command` and `oasis_free_string`.

use std::ffi::CString;
use std::os::raw::c_char;

use oasis_core::terminal::{CommandOutput, Environment};

use crate::handle::{OasisInstance, c_str_to_str, with_instance};
use crate::types::OASIS_CB_COMMAND_EXEC;

/// Execute a terminal command and return the output as a C string.
///
/// The caller must free the returned string with `oasis_free_string`.
/// Returns null on error.
///
/// # Safety
///
/// `handle` must be valid. `cmd` must be a valid null-terminated C string.
///
/// # Thread Safety
///
/// Caller must ensure single-threaded access to the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_send_command(
    handle: *mut OasisInstance,
    cmd: *const c_char,
) -> *mut c_char {
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
}

fn send_command_inner(instance: &mut OasisInstance, cmd_str: &str) -> *mut c_char {
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
        Ok(CommandOutput::Text(text)) => text,
        Ok(CommandOutput::Table { headers, rows }) => {
            let mut out = headers.join(" | ");
            for row in &rows {
                out.push('\n');
                out.push_str(&row.join(" | "));
            }
            out
        },
        Ok(CommandOutput::Clear) => String::new(),
        Ok(CommandOutput::None) => String::new(),
        Ok(CommandOutput::Signal(ref sig)) => {
            use oasis_core::terminal::CommandSignal;
            match sig {
                CommandSignal::BrowserSandbox { enable } => {
                    let state = if *enable { "on" } else { "off" };
                    format!("Browser sandbox: {state}")
                },
                CommandSignal::SkinSwap { name } => {
                    format!("Skin swap to '{name}' not available via FFI.")
                },
                _ => "Not available via FFI.".to_string(),
            }
        },
        Ok(CommandOutput::Multi(outputs)) => {
            let mut parts = Vec::new();
            for output in outputs {
                let text = match output {
                    CommandOutput::Text(t) => t,
                    CommandOutput::Table { headers, rows } => {
                        let mut out = headers.join(" | ");
                        for row in &rows {
                            out.push('\n');
                            out.push_str(&row.join(" | "));
                        }
                        out
                    },
                    CommandOutput::Clear | CommandOutput::None => continue,
                    CommandOutput::Signal(ref sig) => {
                        use oasis_core::terminal::CommandSignal;
                        match sig {
                            CommandSignal::BrowserSandbox { enable } => {
                                let state = if *enable { "on" } else { "off" };
                                format!("Browser sandbox: {state}")
                            },
                            CommandSignal::SkinSwap { name } => {
                                format!("Skin swap to '{name}' not available via FFI.")
                            },
                            _ => "Not available via FFI.".to_string(),
                        }
                    },
                    CommandOutput::Multi(_) => continue,
                };
                parts.push(text);
            }
            parts.join("\n")
        },
        Err(e) => format!("error: {e}"),
    };

    instance.cwd = env.cwd;

    CString::new(output)
        .map(|cs| cs.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Free a string previously returned by `oasis_send_command`.
///
/// # Safety
///
/// `ptr` must be a pointer returned by `oasis_send_command`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        // SAFETY: Reclaiming ownership of CString allocated by `CString::into_raw`.
        drop(unsafe { CString::from_raw(ptr) });
    }
}
