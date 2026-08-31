# Changelog
Todas as mudanças notáveis neste projeto serão documentadas aqui.

O formato é baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e este projeto adere ao [Semantic Versioning](https://semver.org/lang/pt-BR/).

Podem existir falhas ou itens não declarados, causados por falha humana ou por IA, caso encontre por favor relate para ajudar a manter a consistência dos dados.

---

## [Não lançado]

### Adicionado
- **Call stack multi-frame** — o plugin caminha a cadeia de frames do AMX (FRM → endereço de retorno) e entrega nome da função, linha e variáveis de cada frame; o editor navega entre eles e a inspeção segue o frame selecionado.
- **Breakpoints de função** — parar ao entrar numa função pelo nome. O adaptador resolve o nome no bloco `AMX_DBG` e envia ao plugin a união dos breakpoints de linha e de função; nome que não resolve volta como não verificado.
- **Data breakpoints** — pausar quando um valor muda, em globais, locais e **elementos de array** (`arr[3]`). Watches de locais expiram quando o frame dono retorna, para não disparar com o lixo de outro frame no mesmo slot; endereço ilegível não gera disparo.
- **Inspeção rica de arrays e strings** — arrays são expansíveis (cada elemento vira um filho) e arrays de char são resumidos como **string** quando o conteúdo parece texto terminado em zero.
- **`setExpression`** — editar um lvalue (`x = 1`, `arr[i] = 10`) direto no watch ou no console, com o índice podendo ser uma subexpressão. O array inteiro não é editável, só os elementos.
- **`readMemory`** — hex view da memória de dados crua a partir de qualquer variável: cada variável passa a expor um `memoryReference`, e o adaptador devolve os bytes em base64 (encoder próprio, sem dependência nova).
- **Autocomplete** (`completions`) — sugestão de variáveis em escopo no watch e no console.
- **Filtro de exceção** — o editor liga e desliga a pausa em erros de runtime pelo painel de breakpoints.
- **Mais erros de runtime** — `STACKERR` (colisão pilha/heap), `HEAPLOW` (underflow de heap) e `MEMACCESS` (acesso inválido à memória), com simulação fiel ao `amx.c` e conservadora: o rastreio de `pri`/`alt`/`stk`/`hea` perde a confiança ao primeiro opcode não modelado, então nunca há falso-positivo.
- **Avaliador de expressões** para watch/hover — um operador de topo com `+ - * / %` (semântica de truncamento do Pawn) e `== != < > <= >=`, sobre literais, variáveis e `arr[i]`.

### Alterado
- **Mensagens localizadas centralizadas** em `crates/protocol/src/messages` — `Locale`, as 11 `MsgKey` e uma tabela por idioma (pt-BR, en, es, ro, ru), compartilhadas pelo plugin e pelo adaptador. O `match` por chave é exaustivo: idioma incompleto não compila.
- **Primeiro canal request/response do protocolo** — `Command::ReadMemory` ↔ `Event::MemoryData`, correlacionados por `id` e com timeout, sem prender a sessão se o plugin não responder.
- **Documentação reescrita** — `README.md` (com badges de CI, CodeQL, docs, OpenSSF Scorecard, release, downloads, stars e licença), `docs/features.md`, `docs/architecture.md`, `docs/index.md`, `docs/getting-started.md` e a nova página de [localização](docs/i18n.md), com a meta de 50 idiomas.
- **Site da documentação bilíngue** — `mkdocs-static-i18n` com **pt-BR** na raiz e **en-US** em `/en-US/`, as seis páginas traduzidas e o nav por `nav_translations`. O tema do locale `en-US` aponta para `en`, porque o Material só traz tabela de interface para `en`.
- **Comentários do código padronizados em português** — `hook.rs` estava inteiramente em inglês (mais pontos isolados em `plugin_client.rs`, `inspect.rs` e `langs/en.rs`); na passagem, cortado o que apenas repetia a assinatura.

### Corrigido
- Tabela de erros em `docs/runtime-errors.md`: faltavam `OP_CALL_PRI` no STACKERR e `OP_LODB_I`/`OP_STRB_I`/`OP_LIDX_B` no MEMACCESS, todos já checados no plugin.
- Doc de `resolve_data_watches`, que ainda dizia que arrays não eram observáveis — elementos de array passaram a ser.

### Infraestrutura
- **OpenSSF Scorecard** (`.github/workflows/scorecard.yml`) — análise em push no `master`, semanal e manual, com actions pinadas por SHA e permissões mínimas. Publica na API pública do OpenSSF (o que sustenta o badge) e envia o SARIF ao code scanning.

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
