//! Inspeção de variáveis em escopo — a lógica (quais símbolos, como formatar)
//! separada da leitura de memória real (`get_ref`), via o trait [`CellReader`].
//! Assim a coleta é testável com um leitor falso, sem servidor.

use samp::debug::AmxDbg;

use pawnpro_dbg_protocol::Var;

/// Lê uma célula (32 bits) da memória da AMX. A implementação real usa
/// `Amx::get_ref`; nos testes, um mapa em memória.
pub trait CellReader {
    /// Lê a célula no endereço de **data** dado. `None` se inválido.
    fn read_cell(&self, data_addr: i32) -> Option<i32>;
}

/// Coleta as variáveis visíveis no endereço de código `cip`, dado o frame `frm`.
/// Globais usam endereço absoluto; locais/args são relativos ao frame.
#[must_use]
pub fn collect(dbg: &AmxDbg, reader: &impl CellReader, cip: u32, frm: i32) -> Vec<Var> {
    let mut out = Vec::new();
    for sym in dbg.symbols_in_scope(cip) {
        // Effective data-segment address (global vs frame-relative) via the SDK.
        let addr = sym.effective_address(frm);
        let value = if sym.is_array() {
            format_array(sym, addr, reader)
        } else {
            reader.read_cell(addr).map_or_else(
                || "?".to_string(),
                |c| format_scalar(c, dbg.tag_name(sym.tag)),
            )
        };
        out.push(Var {
            name: sym.name.clone(),
            value,
        });
    }
    out
}

/// Formata um valor escalar conforme o tag do símbolo. Em Pawn todo valor é um
/// cell de 32 bits; o tag diz como interpretá-lo:
/// - `Float`: os bits são um `f32` IEEE-754 (senão `96.5` apareceria como o
///   inteiro `1119944704`).
/// - `bool`: `0`/`1` viram `false`/`true`.
/// - demais: inteiro com sinal.
fn format_scalar(cell: i32, tag: Option<&str>) -> String {
    match tag {
        Some("Float") => {
            let f = f32::from_bits(cell.cast_unsigned());
            // Notação enxuta: sem zeros à toa, mas mantendo a parte fracionária.
            format!("{f}")
        }
        Some("bool") => if cell == 0 { "false" } else { "true" }.to_string(),
        _ => cell.to_string(),
    }
}

/// Formato compacto de um array: `[a, b, c, …]` até um limite, evitando despejar
/// arrays enormes na inspeção.
fn format_array(sym: &samp::debug::DbgSymbol, base: i32, reader: &impl CellReader) -> String {
    const MAX: u32 = 8;
    let len = sym.dims.first().map_or(0, |d| d.size);
    let show = len.min(MAX);
    let mut parts = Vec::new();
    for i in 0..show {
        let addr = base.wrapping_add(i32::try_from(i).unwrap_or(0) * 4);
        match reader.read_cell(addr) {
            Some(c) => parts.push(c.to_string()),
            None => parts.push("?".to_string()),
        }
    }
    let ellipsis = if len > show { ", …" } else { "" };
    format!("[{}{}]", parts.join(", "), ellipsis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn format_scalar_by_tag() {
        // Float: os bits de 96.5 (1119944704) viram "96.5", não o inteiro cru.
        let bits_965 = 96.5f32.to_bits().cast_signed();
        assert_eq!(format_scalar(bits_965, Some("Float")), "96.5");
        assert_eq!(format_scalar(0, Some("Float")), "0"); // 0.0 → "0"
        let neg = (-3.25f32).to_bits().cast_signed();
        assert_eq!(format_scalar(neg, Some("Float")), "-3.25");
        // bool.
        assert_eq!(format_scalar(0, Some("bool")), "false");
        assert_eq!(format_scalar(1, Some("bool")), "true");
        assert_eq!(format_scalar(7, Some("bool")), "true"); // !=0 → true
        // Sem tag / tag desconhecido → inteiro com sinal.
        assert_eq!(format_scalar(255, None), "255");
        assert_eq!(format_scalar(-250, Some("Qualquer")), "-250");
    }

    /// Leitor falso: memória de dados como mapa endereço→célula.
    struct FakeMem(HashMap<i32, i32>);
    impl CellReader for FakeMem {
        fn read_cell(&self, addr: i32) -> Option<i32> {
            self.0.get(&addr).copied()
        }
    }

    /// Monta um `AmxDbg` com um global e um local (reusa o encoder do parser).
    fn dbg_with_symbols() -> AmxDbg {
        let mut t = Vec::new();
        // files: 1
        push_u32(&mut t, 0);
        push_cstr(&mut t, "a.pwn");
        // lines: 1
        push_u32(&mut t, 0);
        push_i32(&mut t, 1);
        // symbols: 2 — global "g" @200; local "x" rel -4, escopo [0,40)
        push_symbol(&mut t, 200, 0, 1, "g"); // global var (escopo 0..1)
        push_symbol_local(&mut t, (-4i32).cast_unsigned(), 8, 40, "x");
        // header
        let mut b = Vec::new();
        push_i32(&mut b, i32::try_from(22 + t.len()).unwrap());
        b.extend_from_slice(&samp::debug::AMX_DBG_MAGIC.to_le_bytes());
        b.push(1);
        b.push(1);
        push_i16(&mut b, 0); // flags
        push_i16(&mut b, 1); // files
        push_i16(&mut b, 1); // lines
        push_i16(&mut b, 2); // symbols
        push_i16(&mut b, 0); // tags
        push_i16(&mut b, 0); // automatons
        push_i16(&mut b, 0); // states
        b.extend_from_slice(&t);
        AmxDbg::parse(&b).unwrap()
    }

    fn push_i16(v: &mut Vec<u8>, x: i16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn push_u32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn push_i32(v: &mut Vec<u8>, x: i32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn push_cstr(v: &mut Vec<u8>, s: &str) {
        v.extend_from_slice(s.as_bytes());
        v.push(0);
    }

    fn push_symbol(v: &mut Vec<u8>, addr: u32, cs: u32, ce: u32, name: &str) {
        push_u32(v, addr);
        push_i16(v, 0); // tag
        push_u32(v, cs);
        push_u32(v, ce);
        v.push(1); // ident = variable
        v.push(0); // vclass = global
        push_i16(v, 0); // dim
        push_cstr(v, name);
    }
    fn push_symbol_local(v: &mut Vec<u8>, addr: u32, cs: u32, ce: u32, name: &str) {
        push_u32(v, addr);
        push_i16(v, 0);
        push_u32(v, cs);
        push_u32(v, ce);
        v.push(1); // variable
        v.push(1); // vclass = local
        push_i16(v, 0);
        push_cstr(v, name);
    }

    #[test]
    fn reads_global_and_local() {
        let dbg = dbg_with_symbols();
        let mut mem = HashMap::new();
        mem.insert(200, 99); // global g = 99
        mem.insert(100 - 4, 7); // local x: frm(100) + (-4) = 96 → 7
        let reader = FakeMem(mem);

        let vars = collect(&dbg, &reader, 10, 100);
        let g = vars.iter().find(|v| v.name == "g").unwrap();
        let x = vars.iter().find(|v| v.name == "x").unwrap();
        assert_eq!(g.value, "99");
        assert_eq!(x.value, "7");
    }

    #[test]
    fn local_out_of_scope_is_excluded() {
        let dbg = dbg_with_symbols();
        let reader = FakeMem(HashMap::new());
        // cip antes do escopo do local x [8,40): só o global aparece.
        let vars = collect(&dbg, &reader, 4, 100);
        assert!(vars.iter().any(|v| v.name == "g"));
        assert!(!vars.iter().any(|v| v.name == "x"));
    }
}
