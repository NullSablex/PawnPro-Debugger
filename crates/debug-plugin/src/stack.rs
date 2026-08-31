//! Caminhada da pilha de chamadas (call stack) do AMX — a lógica pura, separada
//! da leitura de memória real (`Amx::read_cell`), via um leitor injetável. Assim
//! a caminhada é testável com um mapa de memória falso, sem servidor.
//!
//! # Layout de frame do AMX
//!
//! A pilha do AMX cresce para BAIXO (endereços menores = mais recente), então os
//! frames dos chamadores ficam em endereços MAIORES. O prólogo `OP_PROC` empilha o
//! `FRM` anterior e aponta `FRM` para o topo; a instrução `OP_CALL` empilhou antes
//! o endereço de retorno. Relativo ao `frm` corrente:
//!
//! ```text
//! [frm]        = FRM do chamador (salvo pelo PROC)
//! [frm + CELL] = endereço de retorno no chamador (empilhado pelo CALL)
//! ```
//!
//! O `amx_Exec` empilha um endereço de retorno `0` antes de entrar no público de
//! entrada; ao chegar nele, `[frm + CELL] == 0` encerra a caminhada.

/// Tamanho de uma cell do AMX (32 bits). O `cip`/`OP_BREAK` do resto do plugin já
/// assume 4 (ver `hook::BREAK_OP_SIZE`).
const CELL: i32 = 4;

/// Teto de profundidade da caminhada — guarda contra uma pilha corrompida (frame
/// que não sobe, ciclo) para não girar sem fim no hook de debug.
const MAX_DEPTH: usize = 128;

/// Caminha a pilha a partir do frame do topo `(top_cip, top_frm)` e devolve os
/// frames `(cip, frm)` do topo (índice 0, onde a VM parou) até o público de
/// entrada. `stp` é o topo da pilha (`Amx::stp`), o limite superior válido de um
/// endereço de dados; `read_cell` lê uma cell do segmento de dados (`None` se
/// inacessível).
///
/// Para cada chamador, o `cip` é o endereço de retorno salvo — um offset de
/// código dentro da função chamadora, que mapeia à linha do ponto de chamada.
#[must_use]
pub fn walk(
    top_cip: u32,
    top_frm: i32,
    stp: i32,
    read_cell: impl Fn(i32) -> Option<i32>,
) -> Vec<(u32, i32)> {
    let mut frames = vec![(top_cip, top_frm)];
    let mut frm = top_frm;

    for _ in 0..MAX_DEPTH {
        // Frame precisa caber na pilha para ler os dois slots do cabeçalho.
        if frm <= 0 || frm + CELL >= stp {
            break;
        }
        let (Some(ret), Some(prev)) = (read_cell(frm + CELL), read_cell(frm)) else {
            break;
        };
        // `amx_Exec` empurra retorno 0 antes do público de entrada: sem chamador.
        if ret <= 0 {
            break;
        }
        frames.push((ret.cast_unsigned(), prev));
        // O frame do chamador deve estar ACIMA (endereço maior) e dentro da pilha;
        // caso contrário a cadeia é inválida e paramos após registrar a linha.
        if prev <= frm || prev >= stp {
            break;
        }
        frm = prev;
    }

    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Monta um leitor de memória falso a partir de pares (endereço, valor).
    fn mem(pairs: &[(i32, i32)]) -> impl Fn(i32) -> Option<i32> {
        let map: HashMap<i32, i32> = pairs.iter().copied().collect();
        move |addr| map.get(&addr).copied()
    }

    #[test]
    fn single_frame_when_return_is_zero() {
        // Público de entrada: [frm+4] = 0 (retorno sentinela do amx_Exec).
        let read = mem(&[(1000, 0), (1004, 0)]);
        let frames = walk(40, 1000, 2000, read);
        assert_eq!(frames, vec![(40, 1000)]);
    }

    #[test]
    fn walks_two_levels() {
        // foo (frm=1000) chamado por main (frm=1500), main é o público de entrada.
        // foo:  [1000]=1500 (FRM de main), [1004]=800 (retorno em main)
        // main: [1500]=1900 (FRM anterior), [1504]=0 (entrada → para)
        let read = mem(&[(1000, 1500), (1004, 800), (1500, 1900), (1504, 0)]);
        let frames = walk(40, 1000, 2000, read);
        assert_eq!(frames, vec![(40, 1000), (800, 1500)]);
    }

    #[test]
    fn walks_three_levels() {
        // bar(1000) ← foo(1400) ← main(1800, entrada).
        let read = mem(&[
            (1000, 1400),
            (1004, 600), // retorno em foo
            (1400, 1800),
            (1404, 300), // retorno em main
            (1800, 1950),
            (1804, 0), // entrada
        ]);
        let frames = walk(64, 1000, 2000, read);
        assert_eq!(frames, vec![(64, 1000), (600, 1400), (300, 1800)]);
    }

    #[test]
    fn stops_on_unreadable_cell() {
        // Sem dados para [1000]/[1004]: só o frame do topo.
        let read = mem(&[]);
        let frames = walk(40, 1000, 2000, read);
        assert_eq!(frames, vec![(40, 1000)]);
    }

    #[test]
    fn stops_when_frame_does_not_climb() {
        // prev (1000) não sobe em relação a frm (1000): registra a linha do
        // chamador e para, sem laço infinito.
        let read = mem(&[(1000, 1000), (1004, 800)]);
        let frames = walk(40, 1000, 2000, read);
        assert_eq!(frames, vec![(40, 1000), (800, 1000)]);
    }

    #[test]
    fn stops_when_frame_out_of_stack() {
        // frm no limite de stp: não há espaço para o cabeçalho do frame.
        let read = mem(&[(1996, 100), (2000, 0)]);
        let frames = walk(40, 1998, 2000, read);
        assert_eq!(frames, vec![(40, 1998)]);
    }
}
