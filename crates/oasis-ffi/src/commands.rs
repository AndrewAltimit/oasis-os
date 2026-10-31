//! Command dispatch: `oasis_send_command` and `oasis_free_string`.

use std::ffi::CString;
use std::os::raw::c_char;

use oasis_core::platform::DesktopPlatform;
use oasis_core::terminal::{CommandOutput, CommandSignal, Environment};
use oasis_core::vfs::GameAssetVfs;

use crate::handle::{OasisInstance, c_str_to_str, ffi_guard, to_c_string_lossy, with_instance};
use crate::types::{OASIS_CB_COMMAND_EXEC, OASIS_CB_SHELL_OUTPUT};

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

    let mut env = command_env(instance.cwd.clone(), &mut instance.vfs, &instance.platform);
    let mut output = match instance.cmd_reg.execute(cmd_str, &mut env) {
        Ok(output) => output_text(output),
        Err(e) => format!("error: {e}"),
    };
    // `2>` redirects consume stderr; what is left is the host's to see.
    let stderr = env.stderr.trim_end();
    if !stderr.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(stderr);
    }

    instance.cwd = env.cwd;

    to_c_string_lossy(output).into_raw()
}

/// The command environment of an instance's shell (no network access).
fn command_env<'a>(
    cwd: String,
    vfs: &'a mut GameAssetVfs,
    platform: &'a DesktopPlatform,
) -> Environment<'a> {
    Environment {
        cwd,
        vfs,
        power: Some(platform),
        time: Some(platform),
        usb: Some(platform),
        network: None,
        tls: None,
        stdin: None,
        stderr: String::new(),
    }
}

/// Run the shell's background work for one `oasis_tick` at instance clock
/// `now_secs`: the next queued `cmd &` job, then the due `cron` jobs. Their
/// output goes to the `OASIS_CB_SHELL_OUTPUT` callback (one call per job).
pub(crate) fn poll_background(instance: &mut OasisInstance, now_secs: f64) {
    let mut outputs = Vec::new();
    if instance.cmd_reg.pending_jobs() > 0 {
        let mut env = command_env(instance.cwd.clone(), &mut instance.vfs, &instance.platform);
        if let Some(out) = instance.cmd_reg.poll_jobs(&mut env) {
            outputs.push(output_text(out));
        }
        instance.cwd = env.cwd;
    }
    // Cron jobs start in `/` and their `cd`s don't move the shell's cwd.
    let mut env = command_env("/".to_string(), &mut instance.vfs, &instance.platform);
    let now = std::time::Duration::from_secs_f64(now_secs.max(0.0));
    let lines = instance.cron.tick(now, &instance.cmd_reg, &mut env);
    if !lines.is_empty() {
        outputs.push(lines.join("\n"));
    }
    for text in outputs.iter().filter(|t| !t.is_empty()) {
        instance.fire_callback(OASIS_CB_SHELL_OUTPUT, text);
    }
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
