# PawnPro Debugger

Debugger visual (DAP) para a linguagem **Pawn** (SA-MP / open.mp), integrado ao
[PawnPro](https://github.com/NullSablex/PawnPro), para um servidor local de
desenvolvimento.

## Recursos

| Recurso | Status | Detalhe |
|---------|:------:|---------|
| Breakpoints simples | ✅ | Por linha. |
| Breakpoints condicionais | ✅ | `var OP valor` (`==` `!=` `<` `>` `<=` `>=`); `int`/`Float:`/`bool:`/hex. |
| Hit count | ✅ | `N`, `==N`, `>=N`, `<=N`, `>N`, `<N`, `%N`. |
| Logpoints | ✅ | Mensagem com `{variável}` interpolada, sem pausar. |
| Step in / over / out | ✅ | |
| Inspeção de variáveis | ✅ | int, `Float:`, `bool:`, array, hex — em escopo. |
| Watch / hover | ✅ | |
| Editar variável | ✅ | Durante a pausa (`setVariable`). |
| Pausar em erro de runtime | ✅ | Divisão por zero e índice fora do limite; pausa na linha, antes do abort. SA-MP e open.mp. |
| Mensagens localizadas | ✅ | pt-BR, en, es, ro, ru (segue o idioma do editor). |
| Call stack multi-frame | ⬜ | Hoje mostra um frame; caminhar a pilha está planejado. |
| Data breakpoints | ⬜ | Pausar quando uma variável muda — em avaliação. |
| Mais erros de runtime | ⬜ | STACKERR / MEMACCESS / HEAPLOW — em avaliação. |

## Estrutura (workspace Cargo)

| Crate | Tipo | Papel |
|-------|------|-------|
| [`crates/protocol`](crates/protocol) | `lib` | Protocolo próprio plugin ↔ adaptador (comandos/eventos em NDJSON sobre TCP). |
| [`crates/debug-plugin`](crates/debug-plugin) | `cdylib` | Plugin carregado pelo servidor: debug hook, breakpoint, step, inspeção de memória. |
| [`crates/dap-adapter`](crates/dap-adapter) | `bin` | Adaptador DAP: fala com o editor e com o plugin. |

O parser do formato `AMX_DBG` (endereço ↔ linha ↔ símbolo) vem do SDK
[`rust-samp`](https://crates.io/crates/rust-samp) (`samp::debug` / `samp_sdk::debug`),
fonte única compartilhada entre o plugin e o adaptador.

A extensão PawnPro embute a integração (`contributes.debuggers` + `launch.json`)
e lança o adaptador; este repositório contém apenas o código Rust do debugger.

## Build

```bash
cargo build   # workspace inteiro
cargo test    # todos os testes
```

## Licença

[GNU AGPL v3 ou posterior](LICENSE) (`AGPL-3.0-or-later`). Trabalhos derivados
devem permanecer sob a mesma licença e ter o código-fonte disponível.
