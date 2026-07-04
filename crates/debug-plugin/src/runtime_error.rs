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

/// Idioma das mensagens de erro, resolvido do locale do editor. Espelha o
/// conjunto da engine LSP (mesma regra `from_str`: prefixo de 2 letras).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    PtBr,
    Es,
    Ru,
    Ro,
    #[default]
    En,
}

impl Locale {
    /// Resolve do código de locale (`pt-BR`, `es`, ...). Desconhecido → inglês.
    #[must_use]
    pub fn from_str(s: &str) -> Self {
        let s = s.to_ascii_lowercase();
        if s.starts_with("pt") {
            Self::PtBr
        } else if s.starts_with("es") {
            Self::Es
        } else if s.starts_with("ru") {
            Self::Ru
        } else if s.starts_with("ro") {
            Self::Ro
        } else {
            Self::En
        }
    }
}

/// Erro de runtime iminente detectado no hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    /// Divisão (ou módulo) por zero.
    DivideByZero,
    /// Índice de array fora do limite (`OP_BOUNDS`).
    Bounds,
}

impl RuntimeError {
    /// Texto curto para o `stopped` (reason "exception") do DAP, no idioma dado.
    #[must_use]
    pub fn message(self, locale: Locale) -> &'static str {
        use Locale::{En, Es, PtBr, Ro, Ru};
        match (self, locale) {
            (RuntimeError::DivideByZero, PtBr) => "divisão por zero",
            (RuntimeError::DivideByZero, Es) => "división por cero",
            (RuntimeError::DivideByZero, Ru) => "деление на ноль",
            (RuntimeError::DivideByZero, Ro) => "împărțire la zero",
            (RuntimeError::DivideByZero, En) => "division by zero",
            (RuntimeError::Bounds, PtBr) => "índice de array fora do limite",
            (RuntimeError::Bounds, Es) => "índice de matriz fuera de límite",
            (RuntimeError::Bounds, Ru) => "индекс массива вне диапазона",
            (RuntimeError::Bounds, Ro) => "index de matrice în afara limitelor",
            (RuntimeError::Bounds, En) => "array index out of bounds",
        }
    }
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

/// Varre as instruções a partir de `start` (offset de código), simulando `pri`/
/// `alt` a partir do estado real (`pri0`/`alt0` no break), até detectar um erro
/// de runtime ou chegar ao fim da linha.
///
/// - `frm`: frame atual da VM, para resolver `LOAD_S_*` (`data[frm + offs]`).
/// - `read_code`: lê uma cell crua do CODE segment (`Amx::read_code`).
/// - `read_data`: lê uma cell do DATA segment (`Amx::read_cell`), para emular os
///   `LOAD`/`LOAD_S` — é o que traz os valores das variáveis, sem os quais a
///   detecção da divisão/bounds por variável não funcionaria.
/// - `decode`: traduz o valor cru no número do opcode (via [`OpcodeMap`]).
///
/// Para no próximo `OP_BREAK` (fim da linha), num opcode de tamanho variável, ou
/// quando algo não decodifica — sempre conservador (não inventa erro).
#[must_use]
pub fn scan_line(
    start: u32,
    pri0: i32,
    alt0: i32,
    frm: i32,
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

        // Checa erro ANTES de aplicar efeito (os operandos são os de agora).
        match op {
            OP_SDIV | OP_UDIV if regs.alt == 0 => return Some(RuntimeError::DivideByZero),
            OP_SDIV_ALT | OP_UDIV_ALT if regs.pri == 0 => return Some(RuntimeError::DivideByZero),
            OP_BOUNDS => {
                let limit = param?;
                if regs.pri.cast_unsigned() > limit.cast_unsigned() {
                    return Some(RuntimeError::Bounds);
                }
            }
            _ => {}
        }

        // Aplica o efeito em pri/alt. `LOAD`/`LOAD_S` leem o data segment (o valor
        // real da variável); os demais que não mexem em pri/alt apenas avançam.
        match op {
            OP_LOAD_PRI => regs.pri = param.and_then(read_data).unwrap_or(regs.pri),
            OP_LOAD_ALT => regs.alt = param.and_then(read_data).unwrap_or(regs.alt),
            OP_LOAD_S_PRI => {
                regs.pri = param.and_then(|o| read_data(frm + o)).unwrap_or(regs.pri);
            }
            OP_LOAD_S_ALT => {
                regs.alt = param.and_then(|o| read_data(frm + o)).unwrap_or(regs.alt);
            }
            OP_CONST_PRI => regs.pri = param.unwrap_or(regs.pri),
            OP_CONST_ALT => regs.alt = param.unwrap_or(regs.alt),
            OP_ZERO_PRI => regs.pri = 0,
            OP_ZERO_ALT => regs.alt = 0,
            OP_MOVE_PRI => regs.pri = regs.alt,
            OP_MOVE_ALT => regs.alt = regs.pri,
            OP_XCHG => std::mem::swap(&mut regs.pri, &mut regs.alt),
            OP_PUSH_PRI => stack.push(Some(regs.pri)),
            OP_PUSH_ALT => stack.push(Some(regs.alt)),
            OP_PUSH_C => stack.push(param),
            // `pop` recupera o último push; valor desconhecido mantém o atual.
            OP_POP_PRI => regs.pri = stack.pop().flatten().unwrap_or(regs.pri),
            OP_POP_ALT => regs.alt = stack.pop().flatten().unwrap_or(regs.alt),
            _ => {}
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

    /// Atalho: varre sem memória de dados (frm=0).
    fn scan(mem: Vec<i32>) -> Option<RuntimeError> {
        scan_line(0, 99, 99, 0, &reader(mem), &no_data, &ident)
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
        let r = scan_line(0, 1, 1, 100, &reader(mem), &read_data, &ident);
        assert_eq!(r, Some(RuntimeError::DivideByZero));
    }

    #[test]
    fn real_bounds_of_variable_index() {
        // load.s.pri -20 (i=5) ; bounds 2  → 5 > 2 estoura.
        let mem = code(&[&[OP_LOAD_S_PRI, -20], &[OP_BOUNDS, 2], &[OP_BREAK]]);
        let read_data = |addr: i32| (addr == 80).then_some(5); // frm(100) - 20
        assert_eq!(
            scan_line(0, 0, 0, 100, &reader(mem), &read_data, &ident),
            Some(RuntimeError::Bounds)
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
    fn locale_resolves_by_prefix() {
        assert_eq!(Locale::from_str("pt-BR"), Locale::PtBr);
        assert_eq!(Locale::from_str("PT"), Locale::PtBr); // case-insensitive
        assert_eq!(Locale::from_str("es"), Locale::Es);
        assert_eq!(Locale::from_str("ru-RU"), Locale::Ru);
        assert_eq!(Locale::from_str("ro"), Locale::Ro);
        assert_eq!(Locale::from_str("en-US"), Locale::En);
        assert_eq!(Locale::from_str("zh"), Locale::En); // desconhecido → inglês
        assert_eq!(Locale::default(), Locale::En);
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
