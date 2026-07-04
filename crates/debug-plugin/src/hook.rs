//! Debug-break handling — the decision layer the SDK now feeds.
//!
//! The VM plumbing (installing the hook, reading `cip`/`frm`, bounds-checked
//! cell read/write) lives in the `samp` SDK: this module only decides whether to
//! pause at a given line and, on a pause, collects variables ([`inspect`]),
//! notifies the adapter ([`bridge`]) and **blocks** until continue/step
//! ([`gate`]).
//!
//! [`on_break`] is invoked from `SampPlugin::on_debug_break` (see `lib.rs`),
//! which the SDK wires up via `samp::plugin::enable_debug_hook`. No hand-written
//! `extern "C"` callback and no manual `*mut AMX` poking anymore.

use std::sync::Mutex;

use samp::debug::AmxDbg;
use samp::prelude::Amx;

use crate::bridge::BRIDGE;
use crate::control::{
    Bp, BreakAction, Controller, StepMode, StopReason, eval_condition, interpolate_log,
};
use crate::gate::Resume;
use crate::inspect::{self, CellReader};
use crate::runtime_error::{self, Locale, OP_NUM_OPCODES, OpcodeMap};
use pawnpro_dbg_protocol::{Breakpoint, Event};

/// Size (bytes) of an AMX instruction — the `cip` in the hook points to the cell
/// following the `OP_BREAK`; we step this back to get the line address.
const BREAK_OP_SIZE: u32 = 4;

/// Execution control (breakpoints/step), shared with the TCP thread.
static STATE: Mutex<Controller> = Mutex::new(Controller::new_const());
/// Debug block of the `.amx` being debugged (loaded in the plugin's `on_load`).
static DBG: Mutex<Option<AmxDbg>> = Mutex::new(None);

/// Context of the CURRENT pause (`amx` ptr, `cip`, `frm`), valid only while the
/// VM is blocked in `on_pause`. The socket thread uses this to apply commands
/// that need the VM (e.g. editing a variable). `amx` as `usize` to be `Send`
/// (the VM thread is stopped, so the pointer stays valid during the pause).
static PAUSE_CTX: Mutex<Option<(usize, u32, i32)>> = Mutex::new(None);

/// Opcode map of the loaded VM, to detect a runtime error before it aborts.
/// `None` until `load_opcode_map` runs (and stays effectively identity for a
/// non-relocated image). Built once per VM at load.
static OPCODE_MAP: Mutex<Option<OpcodeMap>> = Mutex::new(None);

/// Idioma das mensagens de erro (resolvido do locale do editor, via env var).
/// Padrão inglês até `set_locale` rodar no carregamento do plugin.
static LOCALE: Mutex<Locale> = Mutex::new(Locale::En);

/// Define o idioma das mensagens de erro. Chamado no `on_load` a partir de
/// `PAWNPRO_DBG_LOCALE` (que o adaptador propaga do editor).
pub fn set_locale(locale: Locale) {
    if let Ok(mut guard) = LOCALE.lock() {
        *guard = locale;
    }
}

/// Reads cells through the SDK's bounds-checked `Amx::read_cell`, which mirrors
/// `amx_GetAddr`. Lets [`inspect::collect`] stay decoupled from the SDK and
/// testable with a fake reader.
impl CellReader for Amx {
    fn read_cell(&self, data_addr: i32) -> Option<i32> {
        Amx::read_cell(self, data_addr)
    }
}

/// Handles a debug break: reads `cip`/`frm` from the VM and decides the pause
/// reason. Breakpoints (with an optional condition) take priority over step. No
/// panic crosses back into the SDK (the trampoline catches it anyway); locks are
/// taken with `if let Ok`.
///
/// Called from `SampPlugin::on_debug_break`.
pub fn on_break(amx: &Amx) {
    let (Some(raw_cip), Some(frm)) = (amx.cip(), amx.frame()) else {
        return;
    };
    // In the debug hook `cip` already pointed to the instruction AFTER the
    // `OP_BREAK` (the ip advanced one 4-byte cell). The line/breakpoint table
    // uses the address of the break itself, so we step back 4 to match.
    let cip = raw_cip.wrapping_sub(BREAK_OP_SIZE);

    // Runtime-error detection takes priority over breakpoint/step: if the NEXT
    // instruction (`raw_cip`, the one about to execute) will abort the VM, pause
    // now with reason "exception" — the VM's ABORT would otherwise return without
    // calling us again. Source line is still the current break's (`cip`).
    if let Some(err) = detect_runtime_error(amx, raw_cip) {
        if let Ok(mut ctrl) = STATE.lock() {
            ctrl.hit_breakpoint(); // clears any pending step; marks started
        }
        let locale = LOCALE.lock().map(|g| *g).unwrap_or_default();
        on_pause(amx, cip, frm, "exception", Some(err.message(locale)));
        return;
    }

    let reason = {
        let Ok(mut ctrl) = STATE.lock() else { return };
        // Breakpoint decision (condition + hit-count + logpoint) in one place.
        // The condition is evaluated lazily against the in-scope variables.
        match ctrl.on_hit(cip, |expr| eval_breakpoint_condition(amx, cip, frm, expr)) {
            BreakAction::Pause => {
                ctrl.hit_breakpoint();
                Some(StopReason::Breakpoint)
            }
            // Logpoint: emit the (interpolated) message and keep running, but a
            // pending step can still stop us this line.
            BreakAction::Log(template) => {
                emit_logpoint(amx, cip, frm, &template);
                ctrl.should_stop(cip, frm)
            }
            BreakAction::None => ctrl.should_stop(cip, frm),
        }
    };
    if let Some(reason) = reason {
        on_pause(amx, cip, frm, reason_str(reason), None);
    }
}

/// Interpolates a logpoint message with the in-scope variables and sends it to the
/// adapter as an `Output` event (no pause). Mirrors the variable lookup used by
/// breakpoint conditions.
fn emit_logpoint(amx: &Amx, cip: u32, frm: i32, template: &str) {
    let Ok(guard) = DBG.lock() else { return };
    let Some(dbg) = guard.as_ref() else { return };
    let vars = inspect::collect(dbg, amx, cip, frm);
    let lookup = |name: &str| -> Option<String> {
        vars.iter()
            .find(|v| v.name == name)
            .map(|v| v.value.clone())
    };
    let text = interpolate_log(template, &lookup);
    BRIDGE.send(&Event::Output { text });
}

fn reason_str(r: crate::control::StopReason) -> &'static str {
    use crate::control::StopReason::{Breakpoint, Entry, Step};
    match r {
        Breakpoint => "breakpoint",
        Step => "step",
        Entry => "entry",
    }
}

/// Pause: collects variables in scope, notifies the adapter and blocks until
/// continue/step. Runs on the VM thread (the server freezes — expected in dev).
fn on_pause(amx: &Amx, cip: u32, frm: i32, reason: &str, description: Option<&str>) {
    let (line, vars) = match DBG.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(dbg) => (dbg.lookup_line(cip), inspect::collect(dbg, amx, cip, frm)),
            None => (None, Vec::new()),
        },
        Err(_) => (None, Vec::new()),
    };

    // Publish the pause context so the socket thread can edit variables while
    // the VM is blocked just below.
    if let (Ok(mut ctx), Some(ptr)) = (PAUSE_CTX.lock(), amx.amx()) {
        *ctx = Some((ptr.as_ptr() as usize, cip, frm));
    }

    BRIDGE.send(&Event::Paused {
        reason: reason.to_string(),
        line,
        vars,
        description: description.map(str::to_string),
    });

    // Block until the adapter sends continue/step; apply the action to the
    // controller.
    let action = BRIDGE.wait_resume();

    // Leaving the pause: invalidate the context (the VM resumes and the pointers
    // no longer hold).
    if let Ok(mut ctx) = PAUSE_CTX.lock() {
        *ctx = None;
    }
    if let Ok(mut ctrl) = STATE.lock() {
        match action {
            Resume::Continue => ctrl.resume(),
            Resume::Step(mode) => ctrl.request_step(mode, frm),
        }
        // `Run` is the post-continue state; the step was already armed above.
        let _ = StepMode::Run;
    }
}

/// Evaluates a breakpoint condition against the variables in scope at the current
/// `cip`/`frm`. `true` = the condition holds (must pause). Conservative: if the
/// inspection/condition cannot be evaluated, `eval_condition` returns `true`.
fn eval_breakpoint_condition(amx: &Amx, cip: u32, frm: i32, expr: &str) -> bool {
    let Ok(guard) = DBG.lock() else { return true };
    let Some(dbg) = guard.as_ref() else {
        return true;
    };
    let vars = inspect::collect(dbg, amx, cip, frm);
    // Resolve a variable name to its ALREADY FORMATTED value (e.g. "96.5",
    // "true", "12"); `eval_condition` reinterprets it by type. Arrays (value
    // "[...]") do not match as a literal → conservative condition.
    let lookup = |name: &str| -> Option<String> {
        vars.iter()
            .find(|v| v.name == name)
            .map(|v| v.value.clone())
    };
    eval_condition(expr, &lookup)
}

/// Loads the debug block used by inspection (call in the plugin's `on_load`).
pub fn load_debug(dbg: AmxDbg) {
    if let Ok(mut guard) = DBG.lock() {
        *guard = Some(dbg);
    }
}

/// Builds this VM's opcode map (inverse of `amx_opcodelist` for a relocated
/// image) for runtime-error detection. Call once per VM in `on_amx_load`.
pub fn load_opcode_map(amx: &Amx) {
    let map = OpcodeMap::new(amx.opcode_table(OP_NUM_OPCODES));
    if let Ok(mut guard) = OPCODE_MAP.lock() {
        *guard = Some(map);
    }
}

/// Scans the source line starting at `at` (a code-segment offset, the first
/// instruction after the `OP_BREAK`) and checks whether any instruction will
/// abort the VM. Simulates `pri`/`alt` from their real values at the break, since
/// the faulting instruction sits mid-line. `None` = safe / undecodable.
fn detect_runtime_error(amx: &Amx, at: u32) -> Option<runtime_error::RuntimeError> {
    let guard = OPCODE_MAP.lock().ok()?;
    let map = guard.as_ref()?;
    let (pri, alt, frm) = (amx.pri()?, amx.alt()?, amx.frame()?);
    let read_code = |off: u32| amx.read_code(off);
    let read_data = |addr: i32| amx.read_cell(addr);
    let decode = |raw: i32| map.decode(raw);
    runtime_error::scan_line(at, pri, alt, frm, &read_code, &read_data, &decode)
}

/// Updates the breakpoints (address + optional condition) resolved by the
/// adapter.
pub fn set_breakpoints(bps: Vec<Breakpoint>) {
    if let Ok(mut ctrl) = STATE.lock() {
        ctrl.set_breakpoints(bps.into_iter().map(|b| Bp {
            addr: b.addr,
            condition: b.condition,
            hit_condition: b.hit_condition,
            log_message: b.log_message,
            hits: 0,
        }));
    }
}

/// Edits a simple variable in scope at the current pause: writes `value` to its
/// cell via the SDK's bounds-checked `Amx::write_cell`. Returns `Some(value)` on
/// success, `None` if there is no active pause, the variable is not in scope, is
/// an array (unsupported) or the address is inaccessible. Called by the socket
/// thread while the VM is paused.
#[must_use]
pub fn set_variable(name: &str, value: i32) -> Option<i32> {
    let (amx_usize, cip, frm) = (*PAUSE_CTX.lock().ok()?)?;
    // Reconstruct an `Amx` over the paused VM pointer. `write_cell` reads the
    // base/data segment straight from the AMX struct, so the function table is
    // not needed here (0 is fine).
    let amx = Amx::new(amx_usize as *mut samp::raw::types::AMX, 0);

    let guard = DBG.lock().ok()?;
    let dbg = guard.as_ref()?;
    // Find the in-scope symbol with this name; arrays are not editable here.
    let sym = dbg
        .symbols_in_scope(cip)
        .into_iter()
        .find(|s| s.name == name)?;
    if sym.is_array() {
        return None;
    }
    if amx.write_cell(sym.effective_address(frm), value) {
        Some(value)
    } else {
        None
    }
}
