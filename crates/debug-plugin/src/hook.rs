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
use std::sync::atomic::{AtomicBool, Ordering};

use samp::debug::AmxDbg;
use samp::prelude::Amx;

use crate::bridge::BRIDGE;
use crate::control::{
    Bp, BreakAction, Controller, DataWatch, StepMode, StopReason, eval_condition, interpolate_log,
};
use crate::gate::Resume;
use crate::inspect::{self, CellReader};
use crate::runtime_error::{self, Locale, OP_NUM_OPCODES, OpcodeMap};
use crate::stack;
use pawnpro_dbg_protocol::{Breakpoint, Event, Frame};
use samp::debug::VClass;

/// Size (bytes) of an AMX instruction — the `cip` in the hook points to the cell
/// following the `OP_BREAK`; we step this back to get the line address.
const BREAK_OP_SIZE: u32 = 4;

/// Execution control (breakpoints/step), shared with the TCP thread.
static STATE: Mutex<Controller> = Mutex::new(Controller::new_const());
/// Debug block of the `.amx` being debugged (loaded in the plugin's `on_load`).
static DBG: Mutex<Option<AmxDbg>> = Mutex::new(None);

/// Context of the CURRENT pause: the `amx` ptr plus every stack frame's
/// `(cip, frm)` (index 0 = top, where the VM stopped). Valid only while the VM is
/// blocked in `on_pause`. The socket thread uses this to apply commands that need
/// the VM in a specific frame (e.g. editing a variable in the selected frame).
/// `amx` as `usize` to be `Send` (the VM thread is stopped, so the pointer stays
/// valid during the pause).
static PAUSE_CTX: Mutex<Option<PauseCtx>> = Mutex::new(None);

/// Pause context: the paused `amx` pointer (as `usize`) plus each frame's
/// `(cip, frm)`, index 0 = top.
type PauseCtx = (usize, Vec<(u32, i32)>);

/// Opcode map of the loaded VM, to detect a runtime error before it aborts.
/// `None` until `load_opcode_map` runs (and stays effectively identity for a
/// non-relocated image). Built once per VM at load.
static OPCODE_MAP: Mutex<Option<OpcodeMap>> = Mutex::new(None);

/// Idioma das mensagens de erro (resolvido do locale do editor, via env var).
/// Padrão inglês até `set_locale` rodar no carregamento do plugin.
static LOCALE: Mutex<Locale> = Mutex::new(Locale::En);

/// Pausa em erro de runtime ligada? (filtro de exceção do editor). Ligado por
/// padrão; o adaptador desliga via `Command::SetExceptionFilter`.
static RUNTIME_ERRORS: AtomicBool = AtomicBool::new(true);

/// Liga/desliga a pausa em erros de runtime (div-zero, bounds, STACKERR, …).
pub fn set_runtime_errors(on: bool) {
    RUNTIME_ERRORS.store(on, Ordering::Relaxed);
}

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
    if RUNTIME_ERRORS.load(Ordering::Relaxed)
        && let Some(err) = detect_runtime_error(amx, raw_cip)
    {
        if let Ok(mut ctrl) = STATE.lock() {
            ctrl.hit_breakpoint(); // clears any pending step; marks started
        }
        let locale = LOCALE.lock().map(|g| *g).unwrap_or_default();
        on_pause(amx, cip, frm, "exception", Some(err.message(locale)));
        return;
    }

    // Data breakpoints: pausa se uma variável observada mudou de valor desde a
    // última linha. Verificado antes do breakpoint/step (é uma causa distinta de
    // parada); watches de locais expiram quando o frame dono retorna.
    if let Some(name) = check_data_watch(amx, cip, frm) {
        if let Ok(mut ctrl) = STATE.lock() {
            ctrl.hit_breakpoint();
        }
        on_pause(amx, cip, frm, "data breakpoint", Some(&name));
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
    let (frames, ctx) = match DBG.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(dbg) => build_frames(dbg, amx, cip, frm),
            None => (Vec::new(), Vec::new()),
        },
        Err(_) => (Vec::new(), Vec::new()),
    };

    // Publica o contexto da pausa (cip/frm de cada frame) para a thread do socket
    // editar variáveis no frame selecionado enquanto a VM está bloqueada abaixo.
    if let (Ok(mut guard), Some(ptr)) = (PAUSE_CTX.lock(), amx.amx()) {
        *guard = Some((ptr.as_ptr() as usize, ctx));
    }

    BRIDGE.send(&Event::Paused {
        reason: reason.to_string(),
        frames,
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

/// Builds the full call stack at the pause: walks the AMX frame chain and, for
/// each frame, resolves the function name/line from the debug block and collects
/// the variables in scope there. Returns the frames for the protocol plus their
/// `(cip, frm)` contexts in the same order, so [`set_variable`] can target the
/// selected frame.
fn build_frames(dbg: &AmxDbg, amx: &Amx, cip: u32, frm: i32) -> (Vec<Frame>, Vec<(u32, i32)>) {
    let stp = amx.stp().unwrap_or(0);
    let ctx = stack::walk(cip, frm, stp, |addr| amx.read_cell(addr));
    let frames = ctx
        .iter()
        .map(|&(fcip, ffrm)| Frame {
            name: dbg.lookup_function(fcip).unwrap_or("???").to_string(),
            line: dbg.lookup_line(fcip),
            vars: inspect::collect(dbg, amx, fcip, ffrm),
        })
        .collect();
    (frames, ctx)
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
    // Ponteiros de pilha/heap e limites, para detectar STACKERR/HEAPLOW/MEMACCESS.
    let (stk, hea, hlw, stp) = (amx.stack()?, amx.heap()?, amx.hlw()?, amx.stp()?);
    let read_code = |off: u32| amx.read_code(off);
    let read_data = |addr: i32| amx.read_cell(addr);
    let decode = |raw: i32| map.decode(raw);
    runtime_error::scan_line(
        at, pri, alt, frm, stk, hea, hlw, stp, &read_code, &read_data, &decode,
    )
}

/// Verifica os data breakpoints neste passo: devolve o nome da variável observada
/// que mudou de valor (e deve pausar), ou `None`. Barato quando não há watch
/// armado. Para expirar watches de locais, calcula os `frm` vivos caminhando a
/// pilha ([`stack::walk`]) — um frame cujo `frm` sumiu retornou.
fn check_data_watch(amx: &Amx, cip: u32, frm: i32) -> Option<String> {
    // Sem watches: não paga o custo de caminhar a pilha.
    if !STATE.lock().ok()?.has_data_watches() {
        return None;
    }
    let stp = amx.stp().unwrap_or(0);
    let live: Vec<i32> = stack::walk(cip, frm, stp, |a| amx.read_cell(a))
        .into_iter()
        .map(|(_, f)| f)
        .collect();
    STATE
        .lock()
        .ok()?
        .check_data_watches(|a| amx.read_cell(a), |f| live.contains(&f))
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

/// Lê `count` bytes da memória de dados a partir da variável `name` (elemento
/// `index`, se array) no `frame`, mais `offset`, e responde com um
/// `Event::MemoryData` correlacionado por `id`. Vazio se não resolver/ler.
pub fn read_memory(
    id: u64,
    frame: usize,
    name: &str,
    index: Option<usize>,
    offset: i64,
    count: usize,
) {
    let bytes = read_memory_inner(frame, name, index, offset, count).unwrap_or_default();
    BRIDGE.send(&Event::MemoryData { id, bytes });
}

fn read_memory_inner(
    frame: usize,
    name: &str,
    index: Option<usize>,
    offset: i64,
    count: usize,
) -> Option<Vec<u8>> {
    let (amx_usize, frames) = PAUSE_CTX.lock().ok().and_then(|g| g.clone())?;
    let amx = Amx::new(amx_usize as *mut samp::raw::types::AMX, 0);
    let (cip, frm) = *frames.get(frame)?;
    let guard = DBG.lock().ok()?;
    let dbg = guard.as_ref()?;
    let sym = dbg
        .symbols_in_scope(cip)
        .into_iter()
        .find(|s| s.name == name)?;
    let mut base = sym.effective_address(frm);
    if let Some(i) = index {
        if !sym.is_array() {
            return None;
        }
        base = base.wrapping_add(i32::try_from(i).ok()?.wrapping_mul(4));
    }
    let start = i32::try_from(i64::from(base) + offset).ok()?;

    // `read_cell` lê cells de 4 bytes alinhadas; alinha para baixo e pula o resto.
    let aligned = start & !3;
    let skip = usize::try_from(start - aligned).ok()?;
    let mut out = Vec::with_capacity(skip + count);
    let mut addr = aligned;
    while out.len() < skip + count {
        let Some(cell) = amx.read_cell(addr) else {
            break; // endereço inacessível: devolve o que leu até aqui
        };
        out.extend_from_slice(&cell.to_le_bytes());
        addr = addr.wrapping_add(4);
    }
    // Fatia [skip, skip+count) do que foi lido (pode ser menor no fim do segmento).
    let end = (skip + count).min(out.len());
    Some(out.get(skip..end).unwrap_or(&[]).to_vec())
}

/// Arma os data breakpoints pedidos pelo adaptador. Resolve cada `(frame, name)`
/// contra a pausa atual (o frame dá `cip`/`frm`; o símbolo em escopo dá o endereço
/// de dados e a classe global/local) e passa os watches resolvidos ao controlador.
/// Chamado pela thread do socket enquanto a VM está pausada.
pub fn set_data_breakpoints(reqs: Vec<pawnpro_dbg_protocol::DataWatch>) {
    let resolved = resolve_data_watches(reqs);
    if let Ok(mut ctrl) = STATE.lock() {
        ctrl.set_data_watches(resolved);
    }
}

/// Resolve os pedidos `(frame, name)` em [`DataWatch`]s com endereço absoluto,
/// classe (global → nunca expira; local → expira com o frame) e valor inicial.
/// Usa o contexto da pausa atual ([`PAUSE_CTX`]) e o bloco de debug. Símbolos que
/// não estão em escopo ou são arrays são ignorados (arrays ainda não observáveis).
fn resolve_data_watches(reqs: Vec<pawnpro_dbg_protocol::DataWatch>) -> Vec<DataWatch> {
    let Some((amx_usize, frames)) = PAUSE_CTX.lock().ok().and_then(|g| g.clone()) else {
        return Vec::new();
    };
    // Reconstrói um `Amx` sobre a VM pausada só para ler as células iniciais.
    let amx = Amx::new(amx_usize as *mut samp::raw::types::AMX, 0);
    let Ok(guard) = DBG.lock() else {
        return Vec::new();
    };
    let Some(dbg) = guard.as_ref() else {
        return Vec::new();
    };
    reqs.into_iter()
        .filter_map(|req| {
            let (cip, frm) = *frames.get(req.frame)?;
            let sym = dbg
                .symbols_in_scope(cip)
                .into_iter()
                .find(|s| s.name == req.name)?;
            let base = sym.effective_address(frm);
            // Elemento de array (`name[index]`) ou escalar. Arrays só são
            // observáveis por um elemento; escalares, sem índice.
            let (addr, name) = if let Some(i) = req.index {
                if !sym.is_array() {
                    return None;
                }
                let len = usize::try_from(sym.dims.first().map_or(0, |d| d.size)).unwrap_or(0);
                if i >= len {
                    return None;
                }
                let addr = base.wrapping_add(i32::try_from(i).ok()?.wrapping_mul(4));
                (addr, format!("{}[{i}]", req.name))
            } else {
                if sym.is_array() {
                    return None;
                }
                (base, req.name)
            };
            // Global: endereço absoluto, nunca expira. Local: relativo ao frame,
            // expira quando o frame `frm` retorna.
            let frame_frm = (sym.vclass != VClass::Global).then_some(frm);
            let last = amx.read_cell(addr).unwrap_or(0);
            Some(DataWatch {
                addr,
                frame_frm,
                last,
                name,
            })
        })
        .collect()
}

/// Edits a variable in scope in the given stack `frame` (0 = top) at the current
/// pause: writes `value` to its cell via the SDK's bounds-checked `Amx::write_cell`.
/// `index` targets an array element (`arr[index]`); `None` edits a scalar. Returns
/// `Some(value)` on success, `None` if there is no active pause, the frame index is
/// out of range, the variable is not in scope, is a scalar edited with an index (or
/// an array edited without one, or the index is out of bounds), or the address is
/// inaccessible. Called by the socket thread while the VM is paused.
#[must_use]
pub fn set_variable(frame: usize, name: &str, index: Option<usize>, value: i32) -> Option<i32> {
    let (amx_usize, cip, frm) = {
        let guard = PAUSE_CTX.lock().ok()?;
        let (amx_usize, frames) = guard.as_ref()?;
        let (cip, frm) = *frames.get(frame)?;
        (*amx_usize, cip, frm)
    };
    // Reconstruct an `Amx` over the paused VM pointer. `write_cell` reads the
    // base/data segment straight from the AMX struct, so the function table is
    // not needed here (0 is fine).
    let amx = Amx::new(amx_usize as *mut samp::raw::types::AMX, 0);

    let guard = DBG.lock().ok()?;
    let dbg = guard.as_ref()?;
    let sym = dbg
        .symbols_in_scope(cip)
        .into_iter()
        .find(|s| s.name == name)?;

    // Endereço-alvo: elemento `index` de um array, ou a célula de um escalar.
    let addr = if let Some(i) = index {
        if !sym.is_array() {
            return None; // índice pedido em algo que não é array
        }
        let len = usize::try_from(sym.dims.first().map_or(0, |d| d.size)).unwrap_or(0);
        if i >= len {
            return None; // fora do limite do array
        }
        sym.effective_address(frm)
            .wrapping_add(i32::try_from(i).ok()?.wrapping_mul(4))
    } else {
        if sym.is_array() {
            return None; // array precisa de índice (o array inteiro não é editável)
        }
        sym.effective_address(frm)
    };

    amx.write_cell(addr, value).then_some(value)
}
