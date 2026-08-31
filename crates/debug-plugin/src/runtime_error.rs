//! Detecção de erro de runtime ANTES de a VM abortar — pura e testável.
//!
//! A VM AMX aborta erros (divisão por zero, índice fora de faixa) com a macro
//! `ABORT`, que retorna imediatamente de `amx_Exec` sem chamar o debug hook nem
//! preservar o `cip` exato. Para pausar na linha do erro (como Python/PHP), o
//! hook — chamado uma vez por LINHA-fonte (no `OP_BREAK` que abre a linha) —
//! varre as instruções daquela linha simulando os registradores `pri`/`alt` e
//! detecta se alguma vai falhar.
//!
//! ## Por que simular
//! O `OP_BREAK` dispara no INÍCIO da linha; a instrução perigosa está no meio
//! dela, depois de vários `load`/`const`/`push`/`pop` que alteram `pri`/`alt`.
//! Olhar só o registrador no break daria o valor errado. [`scan_line`] reexecuta
//! (sem efeitos colaterais) as instruções que mexem em `pri`/`alt` até a próxima
//! linha, checando cada `OP_*DIV`/`OP_BOUNDS` com os valores corretos.
//!
//! ## A pegadinha da relocação
//! Em servidores computed-goto (GCC/Clang — SA-MP e open.mp) o loader reescreve
//! os opcodes no code segment para o ENDEREÇO do label. [`OpcodeMap`] inverte
//! essa tabela (ponteiro → opcode). Em imagens não-relocadas, o valor já é o
//! número.

use std::collections::HashMap;

/// Números de opcode da VM AMX (ordem do enum em `amx.c`). Só os que o simulador
/// de linha consome (efeito em `pri`/`alt`) ou detecta.
pub const OP_LOAD_PRI: i32 = 1; // pri = data[offs]
pub const OP_LOAD_ALT: i32 = 2; // alt = data[offs]
pub const OP_LOAD_S_PRI: i32 = 3; // pri = data[frm+offs]
pub const OP_LOAD_S_ALT: i32 = 4; // alt = data[frm+offs]
pub const OP_CONST_PRI: i32 = 11;
pub const OP_CONST_ALT: i32 = 12;
pub const OP_MOVE_PRI: i32 = 33; // pri = alt
pub const OP_MOVE_ALT: i32 = 34; // alt = pri
pub const OP_XCHG: i32 = 35;
pub const OP_PUSH_PRI: i32 = 36;
pub const OP_PUSH_ALT: i32 = 37;
pub const OP_PUSH_C: i32 = 39;
pub const OP_POP_PRI: i32 = 42;
pub const OP_POP_ALT: i32 = 43;
pub const OP_SDIV: i32 = 73;
pub const OP_SDIV_ALT: i32 = 74;
pub const OP_UDIV: i32 = 76;
pub const OP_UDIV_ALT: i32 = 77;
pub const OP_ZERO_PRI: i32 = 89;
pub const OP_ZERO_ALT: i32 = 90;
pub const OP_BOUNDS: i32 = 121;
pub const OP_BREAK: i32 = 137;
// Opcodes de endereço/memória e pilha/heap, para os erros STACKERR/HEAPLOW/
// MEMACCESS (números conferidos na `amx_opcodelist` do interpretador).
pub const OP_LOAD_I: i32 = 9; // pri = data[pri]
pub const OP_LODB_I: i32 = 10; // pri = data[pri] (byte/word)
pub const OP_ADDR_PRI: i32 = 13; // pri = frm + offs
pub const OP_ADDR_ALT: i32 = 14; // alt = frm + offs
pub const OP_STOR_I: i32 = 23; // data[alt] = pri
pub const OP_STRB_I: i32 = 24; // data[alt] = pri (byte/word)
pub const OP_LIDX: i32 = 25; // pri = data[pri*4 + alt]
pub const OP_LIDX_B: i32 = 26; // pri = data[(pri<<n) + alt]
pub const OP_IDXADDR: i32 = 27; // pri = pri*4 + alt
pub const OP_IDXADDR_B: i32 = 28; // pri = (pri<<n) + alt
pub const OP_PUSH_R: i32 = 38; // empilha pri, offs vezes
pub const OP_PUSH: i32 = 40; // empilha data[offs]
pub const OP_PUSH_S: i32 = 41; // empilha data[frm+offs]
pub const OP_STACK: i32 = 44; // alt=stk; stk += offs
pub const OP_HEAP: i32 = 45; // alt=hea; hea += offs
pub const OP_PROC: i32 = 46; // empilha frm; frm=stk
pub const OP_CALL: i32 = 49; // empilha retorno; salta
pub const OP_CALL_PRI: i32 = 50;
pub const OP_PUSH_ADR: i32 = 133; // empilha frm+offs
/// Faixa dos opcodes compostos `push2`..`push5` (empilham 2..5 valores).
pub const OP_PUSH2_C: i32 = 138;
pub const OP_PUSH5_ADR: i32 = 153;
/// Margem de segurança pilha↔heap do `amx.c` (`STKMARGIN` = 16 cells).
const STK_MARGIN: i32 = 16 * 4;
/// Total de opcodes (`OP_NUM_OPCODES`) — tamanho da `amx_opcodelist`.
pub const OP_NUM_OPCODES: usize = 158;

/// Nº de cells de parâmetro inline de cada opcode (gerado de `amx_BrowseRelocate`
/// em `amx.c`). `99` = tamanho variável (CASETBL/SWITCH/inválido) → a varredura
/// para por segurança ao encontrá-lo.
#[rustfmt::skip]
const OP_PARAMS: [u8; OP_NUM_OPCODES] = [
    99,1,1,1,1,1,1,1,1,0,1,1,1,1,1,1,1,1,1,1,1,1,1,0,1,0,1,0,1,1,1,1,1,0,0,0,
    0,0,1,1,1,1,0,0,1,1,0,0,0,1,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,0,0,1,1,1,1,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,1,0,0,1,1,0,0,0,0,0,0,0,0,0,0,0,0,1,1,0,
    0,1,1,0,0,0,1,1,0,1,1,1,1,1,0,1,0,0,0,0,0,99,99,0,0,1,0,2,1,0,2,2,2,2,3,
    3,3,3,4,4,4,4,5,5,5,5,2,2,2,2,
];

// Idioma das mensagens: definido uma vez no protocolo (compartilhado com o
// adaptador). Re-exportado para os usos internos do plugin.
pub use pawnpro_dbg_protocol::messages::Locale;
use pawnpro_dbg_protocol::messages::{self, MsgKey};

/// Erro de runtime iminente detectado no hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    /// Divisão (ou módulo) por zero.
    DivideByZero,
    /// Índice de array fora do limite (`OP_BOUNDS`).
    Bounds,
    /// Colisão pilha/heap — a pilha cresceu (ou o heap subiu) até invadir a margem
    /// do outro (`AMX_ERR_STACKERR`, `CHKMARGIN`). É o caso típico de recursão
    /// infinita.
    StackError,
    /// Underflow de heap — liberou o heap abaixo do seu início (`AMX_ERR_HEAPLOW`,
    /// `CHKHEAP`).
    HeapLow,
    /// Acesso inválido à memória — endereço na lacuna entre heap e pilha, ou além
    /// do topo da pilha (`AMX_ERR_MEMACCESS`).
    MemAccess,
}

impl RuntimeError {
    /// Chave da mensagem localizável correspondente.
    #[must_use]
    fn key(self) -> MsgKey {
        match self {
            RuntimeError::DivideByZero => MsgKey::DivideByZero,
            RuntimeError::Bounds => MsgKey::Bounds,
            RuntimeError::StackError => MsgKey::StackError,
            RuntimeError::HeapLow => MsgKey::HeapLow,
            RuntimeError::MemAccess => MsgKey::MemAccess,
        }
    }

    /// Texto curto para o `stopped` (reason "exception") do DAP, no idioma dado.
    #[must_use]
    pub fn message(self, locale: Locale) -> &'static str {
        messages::msg(locale, self.key())
    }
}

/// Endereço de dados inválido, conforme o `VERIFYADDRESS` do `amx.c`: cai na
/// lacuna livre entre o heap (`hea`) e a pilha (`stk`), ou está em/acima do topo
/// da pilha (`stp`) — inclui endereços negativos (viram enormes sem sinal).
#[must_use]
fn mem_invalid(addr: i32, hea: i32, stk: i32, stp: i32) -> bool {
    (addr >= hea && addr < stk) || addr.cast_unsigned() >= stp.cast_unsigned()
}

/// Traduz o valor cru lido do code segment (via `read_code`) no número do opcode.
pub struct OpcodeMap {
    /// `endereço do label → número do opcode`. Vazio = imagem não relocada.
    inverse: HashMap<usize, i32>,
}

impl OpcodeMap {
    /// Constrói o mapa a partir da `amx_opcodelist` (de `Amx::opcode_table`).
    #[must_use]
    pub fn new(opcode_table: Option<Vec<usize>>) -> Self {
        let inverse = opcode_table
            .map(|table| {
                table
                    .into_iter()
                    .enumerate()
                    .map(|(op, addr)| (addr, i32::try_from(op).unwrap_or(-1)))
                    .collect()
            })
            .unwrap_or_default();
        Self { inverse }
    }

    /// Opcode real a partir do valor cru em `code[cip]`. Resolve o endereço de
    /// label (computed-goto) ou aceita um número de opcode pequeno (não relocado).
    /// `None` se não for nenhum dos dois.
    #[must_use]
    pub fn decode(&self, raw: i32) -> Option<i32> {
        if self.inverse.is_empty() {
            return Some(raw);
        }
        if let Some(&op) = self
            .inverse
            .get(&usize::try_from(raw.cast_unsigned()).ok()?)
        {
            return Some(op);
        }
        (0..i32::try_from(OP_NUM_OPCODES).ok()?)
            .contains(&raw)
            .then_some(raw)
    }
}

/// Estado simulado dos registradores durante a varredura de uma linha.
struct Regs {
    pri: i32,
    alt: i32,
}

/// Opcode de controle de fluxo ou que mexe em `stk`/`hea`/`frm` de um jeito que a
/// varredura NÃO modela (saltos, `ret`, `sysreq`, `sctrl`, `switch`). Ao encontrar
/// um deles, o rastreio de `stk`/`hea` deixa de ser confiável — daí para a frente
/// não checamos mais STACKERR/HEAPLOW/MEMACCESS (conservador: não inventa erro).
/// `call`/`call.pri` são tratados à parte (checam antes de virar barreira).
fn is_control_barrier(op: i32) -> bool {
    matches!(op, 32 | 47 | 48 | 120 | 122 | 123 | 128 | 129 | 130 | 135) || (51..=64).contains(&op)
}

/// Varre as instruções a partir de `start` (offset de código), simulando os
/// registradores a partir do estado real no break, até detectar um erro de runtime
/// ou chegar ao fim da linha.
///
/// Os registradores vêm do estado real no break; `stk`/`hea` são rastreados ao
/// longo da linha e checados com as MESMAS condições do `amx.c`
/// (`CHKMARGIN`/`CHKHEAP`/`VERIFYADDRESS`).
///
/// Para no próximo `OP_BREAK`, num opcode de tamanho variável, ou quando algo não
/// decodifica. Ver [`is_control_barrier`] para quando as checagens se desligam.
#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
#[must_use]
pub fn scan_line(
    start: u32,
    pri0: i32,
    alt0: i32,
    frm: i32,
    stk0: i32,
    hea0: i32,
    hlw: i32,
    stp: i32,
    read_code: &impl Fn(u32) -> Option<i32>,
    read_data: &impl Fn(i32) -> Option<i32>,
    decode: &impl Fn(i32) -> Option<i32>,
) -> Option<RuntimeError> {
    const CELL: u32 = 4;
    const MAX_STEPS: usize = 256;

    let mut regs = Regs {
        pri: pri0,
        alt: alt0,
    };
    // `pri`/`alt` só valem para MEMACCESS enquanto forem rastreados exatamente;
    // um opcode que os escreve de forma não modelada zera a confiança.
    let (mut pri_known, mut alt_known) = (true, true);
    // Ponteiros de pilha/heap rastreados; `reliable` cai ao 1º opcode não modelado.
    let (mut stk, mut hea) = (stk0, hea0);
    let mut reliable = true;
    // Pilha simulada (só dos `push` dentro desta linha), para os `pop` casarem o
    // valor certo. Valores desconhecidos (push de algo não rastreado) são `None`.
    let mut stack: Vec<Option<i32>> = Vec::new();
    let mut cip = start;

    for _ in 0..MAX_STEPS {
        let raw = read_code(cip)?;
        let op = decode(raw)?;
        // Fim da linha: o próximo break encerra a varredura.
        if op == OP_BREAK {
            return None;
        }
        let nparams = u32::from(*OP_PARAMS.get(usize::try_from(op).ok()?)?);
        if nparams == 99 {
            return None; // tamanho variável → não dá para avançar com segurança
        }
        // Parâmetro inline (1ª cell após o opcode), quando houver.
        let param = if nparams >= 1 {
            read_code(cip + CELL)
        } else {
            None
        };

        // Checa erro ANTES de aplicar efeito (os operandos são os de agora), na
        // mesma ordem em que o `amx.c` abortaria.
        match op {
            OP_SDIV | OP_UDIV if regs.alt == 0 => return Some(RuntimeError::DivideByZero),
            OP_SDIV_ALT | OP_UDIV_ALT if regs.pri == 0 => return Some(RuntimeError::DivideByZero),
            OP_BOUNDS => {
                let limit = param?;
                if regs.pri.cast_unsigned() > limit.cast_unsigned() {
                    return Some(RuntimeError::Bounds);
                }
            }
            // MEMACCESS: endereço em `pri` (load) ou `alt` (store), ou computado
            // (`lidx`). Só checa com o registrador de endereço confiável.
            OP_LOAD_I | OP_LODB_I if reliable && pri_known => {
                if mem_invalid(regs.pri, hea, stk, stp) {
                    return Some(RuntimeError::MemAccess);
                }
            }
            OP_STOR_I | OP_STRB_I if reliable && alt_known => {
                if mem_invalid(regs.alt, hea, stk, stp) {
                    return Some(RuntimeError::MemAccess);
                }
            }
            OP_LIDX if reliable && pri_known && alt_known => {
                let off = regs.pri.wrapping_mul(4).wrapping_add(regs.alt);
                if mem_invalid(off, hea, stk, stp) {
                    return Some(RuntimeError::MemAccess);
                }
            }
            OP_LIDX_B if reliable && pri_known && alt_known => {
                if let Some(sh) = param {
                    let off = regs
                        .pri
                        .wrapping_shl(sh.cast_unsigned())
                        .wrapping_add(regs.alt);
                    if mem_invalid(off, hea, stk, stp) {
                        return Some(RuntimeError::MemAccess);
                    }
                }
            }
            _ => {}
        }

        // Aplica o efeito: rastreia `pri`/`alt` (valor + confiança), `stk`/`hea`, e
        // checa STACKERR/HEAPLOW nos pontos em que o `amx.c` roda `CHKMARGIN`/
        // `CHKHEAP`. `call` antecipa a checagem do prólogo (`PROC`) do chamado.
        match op {
            OP_LOAD_PRI => {
                regs.pri = param.and_then(read_data).unwrap_or(regs.pri);
                pri_known = param.and_then(read_data).is_some();
            }
            OP_LOAD_ALT => {
                regs.alt = param.and_then(read_data).unwrap_or(regs.alt);
                alt_known = param.and_then(read_data).is_some();
            }
            OP_LOAD_S_PRI => {
                let v = param.and_then(|o| read_data(frm + o));
                regs.pri = v.unwrap_or(regs.pri);
                pri_known = v.is_some();
            }
            OP_LOAD_S_ALT => {
                let v = param.and_then(|o| read_data(frm + o));
                regs.alt = v.unwrap_or(regs.alt);
                alt_known = v.is_some();
            }
            OP_CONST_PRI => {
                regs.pri = param.unwrap_or(regs.pri);
                pri_known = param.is_some();
            }
            OP_CONST_ALT => {
                regs.alt = param.unwrap_or(regs.alt);
                alt_known = param.is_some();
            }
            OP_ZERO_PRI => {
                regs.pri = 0;
                pri_known = true;
            }
            OP_ZERO_ALT => {
                regs.alt = 0;
                alt_known = true;
            }
            OP_MOVE_PRI => {
                regs.pri = regs.alt;
                pri_known = alt_known;
            }
            OP_MOVE_ALT => {
                regs.alt = regs.pri;
                alt_known = pri_known;
            }
            OP_XCHG => {
                std::mem::swap(&mut regs.pri, &mut regs.alt);
                std::mem::swap(&mut pri_known, &mut alt_known);
            }
            // Endereços: `addr` = frm+offs; `idxaddr` = pri*4+alt (ou pri<<n)+alt.
            OP_ADDR_PRI => {
                regs.pri = frm.wrapping_add(param.unwrap_or(0));
                pri_known = param.is_some();
            }
            OP_ADDR_ALT => {
                regs.alt = frm.wrapping_add(param.unwrap_or(0));
                alt_known = param.is_some();
            }
            OP_IDXADDR => {
                regs.pri = regs.pri.wrapping_mul(4).wrapping_add(regs.alt);
                pri_known = pri_known && alt_known;
            }
            OP_IDXADDR_B => {
                if let Some(sh) = param {
                    regs.pri = regs
                        .pri
                        .wrapping_shl(sh.cast_unsigned())
                        .wrapping_add(regs.alt);
                }
                pri_known = pri_known && alt_known && param.is_some();
            }
            // Loads indiretos: após a checagem MEMACCESS acima, `pri` recebe o dado.
            OP_LOAD_I | OP_LODB_I | OP_LIDX | OP_LIDX_B => {
                regs.pri = read_data(regs.pri).unwrap_or(regs.pri);
                pri_known = false; // valor vindo da memória: não rastreado adiante
            }
            OP_PUSH_PRI => {
                stack.push(Some(regs.pri));
                stk -= 4;
            }
            OP_PUSH_ALT => {
                stack.push(Some(regs.alt));
                stk -= 4;
            }
            OP_PUSH_C => {
                stack.push(param);
                stk -= 4;
            }
            OP_PUSH => {
                stack.push(param.and_then(read_data));
                stk -= 4;
            }
            OP_PUSH_S => {
                stack.push(param.and_then(|o| read_data(frm + o)));
                stk -= 4;
            }
            OP_PUSH_ADR => {
                stack.push(param.map(|o| frm.wrapping_add(o)));
                stk -= 4;
            }
            OP_PUSH_R => {
                if let Some(n) = param.filter(|n| *n >= 0) {
                    for _ in 0..n {
                        stack.push(Some(regs.pri));
                    }
                    stk -= 4 * n;
                } else {
                    reliable = false;
                }
            }
            OP_POP_PRI => {
                let v = stack.pop().flatten();
                regs.pri = v.unwrap_or(regs.pri);
                pri_known = v.is_some();
                stk += 4;
            }
            OP_POP_ALT => {
                let v = stack.pop().flatten();
                regs.alt = v.unwrap_or(regs.alt);
                alt_known = v.is_some();
                stk += 4;
            }
            OP_STACK => {
                if let Some(o) = param {
                    regs.alt = stk;
                    alt_known = true;
                    stk = stk.wrapping_add(o);
                    if reliable && hea + STK_MARGIN > stk {
                        return Some(RuntimeError::StackError);
                    }
                } else {
                    reliable = false;
                }
            }
            OP_HEAP => {
                if let Some(o) = param {
                    regs.alt = hea;
                    alt_known = true;
                    hea = hea.wrapping_add(o);
                    if reliable && hea + STK_MARGIN > stk {
                        return Some(RuntimeError::StackError);
                    }
                    if reliable && hea < hlw {
                        return Some(RuntimeError::HeapLow);
                    }
                } else {
                    reliable = false;
                }
            }
            OP_PROC => {
                stk -= 4; // PUSH(frm)
                if reliable && hea + STK_MARGIN > stk {
                    return Some(RuntimeError::StackError);
                }
            }
            OP_CALL | OP_CALL_PRI => {
                // O prólogo (`PROC`) do chamado fará PUSH(retorno)+PUSH(frm) e então
                // `CHKMARGIN`: antecipamos essa checagem (recursão infinita estoura
                // aqui). Depois a varredura não pode seguir para dentro do chamado.
                if reliable && hea + STK_MARGIN > stk - 8 {
                    return Some(RuntimeError::StackError);
                }
                reliable = false;
            }
            // `push2`..`push5`: empilham N valores (efeito só em `stk`, N cells).
            OP_PUSH2_C..=OP_PUSH5_ADR => {
                let n = 2 + (op - OP_PUSH2_C) / 4;
                for _ in 0..n {
                    stack.push(None);
                }
                stk -= 4 * n;
            }
            _ if is_control_barrier(op) => {
                reliable = false;
                pri_known = false;
                alt_known = false;
            }
            // Qualquer outro opcode pode escrever `pri`/`alt` de forma não modelada
            // (aritmética etc.): zera a confiança neles (mantém `stk`/`hea`, que
            // esses opcodes não tocam).
            _ => {
                pri_known = false;
                alt_known = false;
            }
        }

        cip += CELL * (1 + nparams);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monta um "code segment" a partir de uma lista de (opcode, params...).
    fn code(instrs: &[&[i32]]) -> Vec<i32> {
        instrs.iter().flat_map(|i| i.iter().copied()).collect()
    }

    /// Leitor sobre o vetor (offset em bytes; cada cell = 4 bytes).
    fn reader(mem: Vec<i32>) -> impl Fn(u32) -> Option<i32> {
        move |off: u32| {
            off.is_multiple_of(4)
                .then(|| usize::try_from(off / 4).ok())
                .flatten()
                .and_then(|i| mem.get(i).copied())
        }
    }

    /// Decode identidade (imagem não relocada nos testes).
    fn ident(raw: i32) -> Option<i32> {
        (0..i32::try_from(OP_NUM_OPCODES).unwrap())
            .contains(&raw)
            .then_some(raw)
    }

    /// `read_data` que devolve `None` (testes sem memória de dados).
    fn no_data(_: i32) -> Option<i32> {
        None
    }

    /// Limites de pilha/heap "folgados" para os testes de DIV/BOUNDS, em que não
    /// se quer disparar STACKERR/HEAPLOW/MEMACCESS: pilha bem acima do heap, heap
    /// no fundo, topo distante.
    const STK: i32 = 0x1_0000;
    const HEA: i32 = 0;
    const HLW: i32 = 0;
    const STP: i32 = 0x10_0000;

    /// Atalho: varre sem memória de dados (frm=0), com limites folgados.
    fn scan(mem: Vec<i32>) -> Option<RuntimeError> {
        scan_line(
            0,
            99,
            99,
            0,
            STK,
            HEA,
            HLW,
            STP,
            &reader(mem),
            &no_data,
            &ident,
        )
    }

    #[test]
    fn detects_divide_by_zero_mid_line() {
        // const.alt 0 ; sdiv ; break  → alt=0 na divisão.
        assert_eq!(
            scan(code(&[&[OP_CONST_ALT, 0], &[OP_SDIV], &[OP_BREAK]])),
            Some(RuntimeError::DivideByZero)
        );
    }

    #[test]
    fn divide_ok_when_divisor_nonzero() {
        assert_eq!(
            scan(code(&[&[OP_CONST_ALT, 5], &[OP_SDIV], &[OP_BREAK]])),
            None
        );
    }

    #[test]
    fn sdiv_alt_uses_pri_as_divisor() {
        // sdiv.alt aborta se pri==0.
        assert_eq!(
            scan(code(&[&[OP_ZERO_PRI], &[OP_SDIV_ALT], &[OP_BREAK]])),
            Some(RuntimeError::DivideByZero)
        );
    }

    #[test]
    fn detects_bounds_overflow() {
        // const.pri 5 ; bounds 4  → 5 > 4 (unsigned) estoura.
        assert_eq!(
            scan(code(&[&[OP_CONST_PRI, 5], &[OP_BOUNDS, 4], &[OP_BREAK]])),
            Some(RuntimeError::Bounds)
        );
    }

    #[test]
    fn bounds_ok_within_limit() {
        assert_eq!(
            scan(code(&[&[OP_CONST_PRI, 3], &[OP_BOUNDS, 4], &[OP_BREAK]])),
            None
        );
    }

    #[test]
    fn stops_at_next_break_without_error() {
        // O sdiv está APÓS o break (outra linha) → não detecta.
        assert_eq!(
            scan(code(&[&[OP_CONST_PRI, 1], &[OP_BREAK], &[OP_SDIV]])),
            None
        );
    }

    #[test]
    fn xchg_and_move_track_registers() {
        // zero.pri (pri=0) ; move.alt (alt=pri=0) ; sdiv (divisor alt=0) → zero.
        assert_eq!(
            scan(code(&[
                &[OP_ZERO_PRI],
                &[OP_MOVE_ALT],
                &[OP_SDIV],
                &[OP_BREAK]
            ])),
            Some(RuntimeError::DivideByZero)
        );
    }

    #[test]
    fn real_division_of_variables_via_loads_and_stack() {
        // Reproduz FIELMENTE `c = a / b` (b=0) como o pawncc 3.10 gera:
        //   load.s.pri -4 (a) ; push.pri ; load.s.pri -8 (b) ; pop.alt ; sdiv.alt
        // sdiv.alt computa alt/pri e aborta se pri==0. Após a sequência:
        //   pop.alt → alt = a (do push) ; pri = b (2º load) ; divisor = pri = b = 0.
        let mem = code(&[
            &[OP_LOAD_S_PRI, -4], // pri = a
            &[OP_PUSH_PRI],       // empilha a
            &[OP_LOAD_S_PRI, -8], // pri = b
            &[OP_POP_ALT],        // alt = a
            &[OP_SDIV_ALT],       // divisor = pri = b = 0 → aborta
            &[OP_BREAK],
        ]);
        // frm=100: data[96]=a=100, data[92]=b=0.
        let read_data = |addr: i32| match addr {
            96 => Some(100), // frm-4 = a
            92 => Some(0),   // frm-8 = b
            _ => None,
        };
        let r = scan_line(
            0,
            1,
            1,
            100,
            STK,
            HEA,
            HLW,
            STP,
            &reader(mem),
            &read_data,
            &ident,
        );
        assert_eq!(r, Some(RuntimeError::DivideByZero));
    }

    #[test]
    fn real_bounds_of_variable_index() {
        // load.s.pri -20 (i=5) ; bounds 2  → 5 > 2 estoura.
        let mem = code(&[&[OP_LOAD_S_PRI, -20], &[OP_BOUNDS, 2], &[OP_BREAK]]);
        let read_data = |addr: i32| (addr == 80).then_some(5); // frm(100) - 20
        assert_eq!(
            scan_line(
                0,
                0,
                0,
                100,
                STK,
                HEA,
                HLW,
                STP,
                &reader(mem),
                &read_data,
                &ident
            ),
            Some(RuntimeError::Bounds)
        );
    }

    /// Varre com pilha/heap/limites explícitos (frm=0, sem memória de dados).
    fn scan_stk(
        pri: i32,
        alt: i32,
        stk: i32,
        hea: i32,
        hlw: i32,
        stp: i32,
        mem: Vec<i32>,
    ) -> Option<RuntimeError> {
        scan_line(
            0,
            pri,
            alt,
            0,
            stk,
            hea,
            hlw,
            stp,
            &reader(mem),
            &no_data,
            &ident,
        )
    }

    #[test]
    fn detects_stack_heap_collision_on_stack_op() {
        // stk=1100, hea=1000; `stack -56` → stk=1044; hea+64=1064 > 1044 → colisão.
        assert_eq!(
            scan_stk(
                0,
                0,
                1100,
                1000,
                0,
                0x10_0000,
                code(&[&[OP_STACK, -56], &[OP_BREAK]])
            ),
            Some(RuntimeError::StackError)
        );
        // Folga suficiente (stk=2000): sem colisão.
        assert_eq!(
            scan_stk(
                0,
                0,
                2000,
                1000,
                0,
                0x10_0000,
                code(&[&[OP_STACK, -56], &[OP_BREAK]])
            ),
            None
        );
    }

    #[test]
    fn detects_stack_overflow_on_recursive_call() {
        // stk=1064, hea=1000; o PROC do chamado fará stk-8=1056; hea+64=1064 > 1056.
        assert_eq!(
            scan_stk(
                0,
                0,
                1064,
                1000,
                0,
                0x10_0000,
                code(&[&[OP_CALL, 0], &[OP_BREAK]])
            ),
            Some(RuntimeError::StackError)
        );
    }

    #[test]
    fn detects_heap_underflow_on_heap_release() {
        // hea=1000, hlw=1000; `heap -4` → hea=996 < hlw → underflow (pilha folgada).
        assert_eq!(
            scan_stk(
                0,
                0,
                0x10_0000,
                1000,
                1000,
                0x20_0000,
                code(&[&[OP_HEAP, -4], &[OP_BREAK]])
            ),
            Some(RuntimeError::HeapLow)
        );
        // Liberação dentro do limite (hlw=900): ok.
        assert_eq!(
            scan_stk(
                0,
                0,
                0x10_0000,
                1000,
                900,
                0x20_0000,
                code(&[&[OP_HEAP, -4], &[OP_BREAK]])
            ),
            None
        );
    }

    #[test]
    fn detects_mem_access_in_heap_stack_gap() {
        // pri=5000 na lacuna [hea=1000, stk=8000) → load.i inválido.
        assert_eq!(
            scan_stk(
                0,
                0,
                8000,
                1000,
                0,
                0x10_0000,
                code(&[&[OP_CONST_PRI, 5000], &[OP_LOAD_I], &[OP_BREAK]])
            ),
            Some(RuntimeError::MemAccess)
        );
        // pri=500 é global (abaixo do heap) → endereço válido.
        assert_eq!(
            scan_stk(
                0,
                0,
                8000,
                1000,
                0,
                0x10_0000,
                code(&[&[OP_CONST_PRI, 500], &[OP_LOAD_I], &[OP_BREAK]])
            ),
            None
        );
    }

    #[test]
    fn mem_access_skipped_when_address_unknown() {
        // const.pri 5000 (na lacuna) ; add (opcode não modelado, zera a confiança) ;
        // load.i → como `pri` deixou de ser rastreado, NÃO acusa (conservador).
        const OP_ADD: i32 = 78;
        assert_eq!(
            scan_stk(
                0,
                0,
                8000,
                1000,
                0,
                0x10_0000,
                code(&[&[OP_CONST_PRI, 5000], &[OP_ADD], &[OP_LOAD_I], &[OP_BREAK]])
            ),
            None
        );
    }

    #[test]
    fn no_check_after_call_barrier() {
        // Após um `call` (barreira: pilha deixa de ser confiável), um `stack` que
        // colidiria NÃO é reportado — evita falso-positivo com fluxo não seguido.
        // call não estoura aqui (pilha bem folgada).
        assert_eq!(
            scan_stk(
                0,
                0,
                0x10_0000,
                0,
                0,
                0x20_0000,
                code(&[&[OP_CALL, 0], &[OP_STACK, -0x0F_FFF0], &[OP_BREAK]])
            ),
            None
        );
    }

    #[test]
    fn opcode_map_identity_when_not_relocated() {
        let map = OpcodeMap::new(None);
        assert_eq!(map.decode(OP_SDIV), Some(OP_SDIV));
    }

    #[test]
    fn opcode_map_inverts_relocated_table() {
        let table: Vec<usize> = (0..OP_NUM_OPCODES).map(|i| 0x1_0000 + i * 8).collect();
        let map = OpcodeMap::new(Some(table));
        let sdiv_addr = 0x1_0000 + (OP_SDIV as usize) * 8;
        assert_eq!(map.decode(i32::try_from(sdiv_addr).unwrap()), Some(OP_SDIV));
        // Valor pequeno cai no fallback de número cru.
        assert_eq!(map.decode(OP_BOUNDS), Some(OP_BOUNDS));
        // Endereço fora da tabela e fora da faixa → None.
        assert_eq!(map.decode(0x9999), None);
    }

    #[test]
    fn message_localized() {
        assert_eq!(
            RuntimeError::DivideByZero.message(Locale::PtBr),
            "divisão por zero"
        );
        assert_eq!(
            RuntimeError::DivideByZero.message(Locale::En),
            "division by zero"
        );
        assert_eq!(
            RuntimeError::Bounds.message(Locale::En),
            "array index out of bounds"
        );
        assert_eq!(
            RuntimeError::Bounds.message(Locale::Es),
            "índice de matriz fuera de límite"
        );
    }
}
