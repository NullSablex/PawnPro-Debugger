# Changelog
Todas as mudanças notáveis neste projeto serão documentadas aqui.

O formato é baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e este projeto adere ao [Semantic Versioning](https://semver.org/lang/pt-BR/).

Podem existir falhas ou itens não declarados, causados por falha humana ou por IA, caso encontre por favor relate para ajudar a manter a consistência dos dados.

---

## [0.1.0] - 04/07/2026

Primeiro pré-lançamento (pre-release).

### Adicionado
- **Arquitetura em 3 crates** — `protocol` (IPC plugin↔adaptador em NDJSON sobre socket local), `debug-plugin` (`cdylib` carregado pelo servidor: debug hook, breakpoint, step, inspeção) e `dap-adapter` (binário que fala DAP com o editor e o protocolo próprio com o plugin).
- **Breakpoints** — simples, **condicionais** (`var OP valor`, com `==`/`!=`/`<`/`>`/`<=`/`>=`, suportando `int`/`Float:`/`bool:`/hex) e por **contagem de acertos** (`hitCondition`: `N`, `==N`, `>=N`, `<=N`, `>N`, `<N`, `%N`).
- **Logpoints** — em vez de pausar, interpolam a mensagem (trechos `{expr}` viram o valor da variável) e a emitem no console do editor.
- **Step** — in, over e out.
- **Inspeção** — variáveis em escopo (inteiro, `Float:`, `bool:`, array e hex), painel de watch, avaliação por hover e **edição de variável** durante a pausa (`setVariable`).
- **Pausar em erro de runtime** — o hook decodifica a próxima instrução da linha (simulando os registradores `pri`/`alt`) e pausa com `reason: "exception"` **antes** de a VM abortar, em **divisão por zero** (`OP_SDIV`/`OP_UDIV`) e **índice de array fora do limite** (`OP_BOUNDS`). Portável entre SA-MP e open.mp: usa a tabela de opcodes obtida em runtime para lidar com a relocação por computed-goto dos servidores.
- **Mensagens localizadas** — os erros de runtime seguem o idioma do editor (pt-BR, en, es, ro, ru), recebido do adaptador via `PAWNPRO_DBG_LOCALE`.
- **Integração com a extensão PawnPro** — a extensão recompila o gamemode com `-d3`, faz o preflight do plugin, sobe o servidor e conecta o adaptador; o `stackTrace` ancora o frame no arquivo-fonte mesmo sem breakpoints.

### Detalhes técnicos
- O parser do formato `AMX_DBG` (endereço ↔ linha ↔ símbolo ↔ função) vem do SDK [`rust-samp`](https://crates.io/crates/rust-samp) (`samp::debug` / `samp_sdk::debug`), fonte única compartilhada entre plugin e adaptador.
- As primitivas de VM usadas na detecção de erro (`Amx::pri`/`alt`/`read_code`/`opcode_table`) também vêm do SDK.
- Edição **2024**, `resolver = "3"`. O plugin é compilado para a arquitetura do servidor (SA-MP/open.mp são 32-bit → `i686-unknown-linux-gnu`); o adaptador roda na arquitetura do host.
- Licenciado sob **GNU AGPL v3 ou posterior** (`AGPL-3.0-or-later`).
