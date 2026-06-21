# PawnPro Debugger

Debugger visual (DAP) para a linguagem **Pawn** (SA-MP / open.mp), integrado ao
[PawnPro](https://github.com/NullSablex/PawnPro). Permite breakpoints, step,
inspeção de variáveis e call stack em um servidor local de desenvolvimento.

## Estrutura (workspace Cargo)

| Crate | Tipo | Papel |
|-------|------|-------|
| [`crates/amxdbg`](crates/amxdbg) | `lib` | Parser do formato de debug `AMX_DBG` (endereço ↔ linha ↔ símbolo). |
| [`crates/debug-plugin`](crates/debug-plugin) | `cdylib` | Plugin carregado pelo servidor: debug hook, breakpoint, step, inspeção de memória. |
| [`crates/dap-adapter`](crates/dap-adapter) | `bin` | Adaptador DAP: fala com o editor e com o plugin. |

A extensão PawnPro embute a integração (`contributes.debuggers` + `launch.json`)
e lança o adaptador; este repositório contém apenas o código Rust do debugger.

## Build

```bash
cargo build            # workspace inteiro
cargo test -p amxdbg   # testes do parser
```
