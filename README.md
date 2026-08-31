<h1 align="center">PawnPro Debugger</h1>

<p align="center">
  Debugger visual (DAP) para a linguagem <strong>Pawn</strong> (SA-MP / open.mp).
</p>

<p align="center">
  <a href="https://github.com/NullSablex/PawnPro-Debugger/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/NullSablex/PawnPro-Debugger/ci.yml?branch=master&label=CI&logo=github"></a>
  <a href="https://github.com/NullSablex/PawnPro-Debugger/actions/workflows/codeql.yml"><img alt="CodeQL" src="https://img.shields.io/github/actions/workflow/status/NullSablex/PawnPro-Debugger/codeql.yml?branch=master&label=CodeQL&logo=github"></a>
  <a href="https://pawnpro-debugger.nullsablex.com/"><img alt="Docs" src="https://img.shields.io/github/actions/workflow/status/NullSablex/PawnPro-Debugger/docs.yml?branch=master&label=docs&logo=materialformkdocs"></a>
  <a href="https://github.com/NullSablex/PawnPro-Debugger/releases"><img alt="Release" src="https://img.shields.io/github/v/release/NullSablex/PawnPro-Debugger?include_prereleases&label=release&logo=github"></a>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-edition%202024-000000?logo=rust">
  <a href="LICENSE"><img alt="Licença" src="https://img.shields.io/badge/licen%C3%A7a-AGPL--3.0--or--later-blue"></a>
</p>

<p align="center">
  <a href="https://pawnpro-debugger.nullsablex.com/">Documentação</a> ·
  <a href="https://pawnpro-debugger.nullsablex.com/getting-started/">Começando</a> ·
  <a href="https://github.com/NullSablex/PawnPro-Debugger/releases">Releases</a> ·
  <a href="https://github.com/NullSablex/PawnPro">Extensão PawnPro</a>
</p>

---

Depure código Pawn direto no editor, num servidor local de desenvolvimento:
breakpoints, step, inspeção e edição de variáveis, e **pausa na linha exata de um
erro de runtime** — antes de a VM abortar. Integrado à extensão
[PawnPro](https://github.com/NullSablex/PawnPro), que lança o adaptador e cuida do
ciclo (recompilar, subir o servidor, conectar).

## Recursos

| Recurso | Detalhe |
|---------|---------|
| Breakpoints | Por linha, **condicionais** (`var OP valor`) e por **contagem de acertos** (`N`, `==N`, `>=N`, `<=N`, `>N`, `<N`, `%N`). |
| Breakpoints de função | Parar ao entrar numa função pelo nome. |
| Logpoints | Mensagem com `{expressão}` interpolada, sem pausar. |
| Step | In, over e out. |
| Call stack multi-frame | Caminha a cadeia de frames (FRM→retorno): função, linha e variáveis por frame. |
| Inspeção de variáveis | `int`, `Float:`, `bool:` e hex; arrays **expansíveis** (elementos como filhos) e arrays de char como **string**. |
| Watch / hover | Expressões com `+ - * / %` e comparações, variáveis e `arr[i]`. |
| Autocomplete | Sugestão de variáveis em escopo no watch e no console. |
| Editar variável | Durante a pausa, por painel (`setVariable`) ou por expressão (`setExpression`), inclusive elementos de array (`arr[i] = 10`). |
| Data breakpoints | Pausar quando um valor muda — globais, locais e **elementos de array**; watches de locais expiram ao retornar o frame. |
| Ler memória | Hex view da memória de dados crua a partir de qualquer variável (`readMemory`). |
| Pausar em erro de runtime | Divisão por zero, índice fora do limite, colisão pilha/heap (`STACKERR`), underflow de heap (`HEAPLOW`) e acesso inválido à memória (`MEMACCESS`) — na linha, antes do abort. SA-MP e open.mp. |
| Mensagens localizadas | pt-BR, en, es, ro, ru — seguem o idioma do editor. |

Detalhes em [Recursos](https://pawnpro-debugger.nullsablex.com/features/) e, para a
pausa no erro, em [Como funciona a pausa no erro](https://pawnpro-debugger.nullsablex.com/runtime-errors/).

## Estrutura (workspace Cargo)

| Crate | Tipo | Papel |
|-------|------|-------|
| [`crates/protocol`](crates/protocol) | `lib` | Protocolo próprio plugin ↔ adaptador (NDJSON sobre socket local) e as mensagens localizadas. |
| [`crates/debug-plugin`](crates/debug-plugin) | `cdylib` | Plugin carregado pelo servidor: debug hook, breakpoints, step, inspeção e leitura da memória da VM. |
| [`crates/dap-adapter`](crates/dap-adapter) | `bin` | Adaptador DAP: fala com o editor e com o plugin. |

O parser do formato `AMX_DBG` (endereço ↔ linha ↔ símbolo) e as primitivas de VM
vêm do SDK [`rust-samp`](https://crates.io/crates/rust-samp) (`samp::debug` /
`samp_sdk::debug`), fonte única compartilhada entre o plugin e o adaptador.

Este repositório contém apenas o código Rust do debugger; a integração com o
editor (`contributes.debuggers` + `launch.json`) fica na extensão PawnPro.

## Build

```bash
cargo build    # workspace inteiro
cargo test     # todos os testes
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
```

O plugin é compilado para a arquitetura do servidor (SA-MP e open.mp são 32-bit →
`i686-unknown-linux-gnu`); o adaptador, para a do host.

## Licença

[GNU AGPL v3 ou posterior](LICENSE) (`AGPL-3.0-or-later`). Trabalhos derivados
devem permanecer sob a mesma licença e ter o código-fonte disponível.
