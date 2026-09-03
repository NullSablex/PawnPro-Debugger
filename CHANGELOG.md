# Changelog
Todas as mudanças notáveis neste projeto serão documentadas aqui.

O formato é baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e este projeto adere ao [Semantic Versioning](https://semver.org/lang/pt-BR/).

Podem existir falhas ou itens não declarados, causados por falha humana ou por IA, caso encontre por favor relate para ajudar a manter a consistência dos dados.

---

## [0.2.1] - 03/09/2026

Correções no reinício da depuração e nos breakpoints.

### Corrigido
- **Reiniciar deixa de encerrar a sessão** — o restart relançava o adaptador inteiro, e como o servidor é processo filho dele tudo caía junto: o editor abria uma sessão nova, com os breakpoints reresolvidos do zero. Agora o processo é trocado por baixo e a sessão continua viva. A identidade passa a ser o canal **nomeado** pela `session`, não o parentesco de processo — subindo outro servidor com a mesma `session`, o plugin reabre o mesmo socket e o cliente reconecta pelo retry que já existia
- **Breakpoints chegam ao plugin** — o editor manda `setBreakpoints` antes do `launch`, e a fila que os guardava era descartada quando o cliente do plugin era trocado. Era ali que os marcadores estavam esperando
- **Breakpoints reresolvidos contra o `.amx` novo** — o binário costuma ser recompilado entre uma sessão e outra, que é o motivo mais comum de reiniciar, e o mapa linha↔endereço muda junto. Sem recarregar o bloco de debug, a VM parava no lugar errado
- **Breakpoints armados no servidor novo** — depois da troca de processo eles existiam no adaptador, mas nunca eram enviados ao plugin recém-conectado
- **Console lido em Windows-1252** — a saída do servidor vinha nessa codificação e era lida como UTF-8: cada acento virava um caractere de substituição no meio das mensagens

### Adicionado
- **Recompilação no reinício** — reiniciar com o fonte alterado deixava o servidor subir com o binário velho. O adaptador compara as datas e pede a recompilação por evento; compilar é atribuição da extensão, que conhece o compilador e as flags. A verificação fica no ponto por onde todo restart passa, venha do botão nativo, da tecla ou da paleta
- **Plugin e adaptador se identificam ao conectar** — o diagnóstico passa a dizer qual versão está de cada lado quando as duas divergem

---

## [0.2.0] - 01/09/2026

Segundo pré-lançamento. Amplia o conjunto DAP suportado: a depuração deixa de ser
"breakpoint e inspeção" e passa a cobrir call stack, data breakpoints, edição por
expressão, leitura de memória e mais três classes de erro de runtime.

### Adicionado
- **Call stack multi-frame** — caminha a cadeia de frames do AMX (FRM → endereço de retorno) e entrega nome da função, linha e variáveis de cada frame. O editor navega entre os frames e a inspeção segue o frame selecionado.
- **Data breakpoints** — pausar quando um valor muda, em globais, locais e **elementos de array** (`arr[3]`). Watches de locais expiram quando o frame dono retorna, para não dispararem com o conteúdo de outro frame no mesmo slot.
- **Breakpoints de função** — parar ao entrar numa função pelo nome; um nome que não resolve volta ao editor como não verificado, sem quebrar a sessão.
- **Três novos erros de runtime** — `STACKERR` (colisão pilha/heap), `HEAPLOW` (underflow de heap) e `MEMACCESS` (acesso inválido à memória), somando-se à divisão por zero e ao índice fora do limite. A simulação é fiel ao `amx.c` e conservadora: o rastreio dos registradores perde a confiança ao primeiro opcode não modelado, então não há falso-positivo.
- **Filtro de exceção** — o editor liga e desliga a pausa em erros de runtime pelo painel de breakpoints.
- **Inspeção de arrays e strings** — arrays são expansíveis (cada elemento vira um filho) e arrays de char são mostrados como **string** quando o conteúdo é texto terminado em zero.
- **Edição por expressão** (`setExpression`) — `x = 1` ou `arr[i] = 10` direto no watch ou no console, com o índice podendo ser uma subexpressão. O array inteiro não é editável, só os elementos.
- **Leitura de memória** (`readMemory`) — hex view da memória de dados crua a partir de qualquer variável, que agora expõe um `memoryReference`.
- **Expressões no watch e no hover** — um operador de topo com `+ - * / %` (seguindo o truncamento do Pawn) e `== != < > <= >=`, sobre literais, variáveis e `arr[i]`.
- **Autocomplete** — sugestão de variáveis em escopo no watch e no console.
- **Documentação em inglês** — o site passa a ter **pt-BR** na raiz e **en-US** em `/en-US/`, com todas as páginas traduzidas e o menu localizado.

### Alterado
- **Mensagens do adaptador agora são localizadas** — antes só os erros de runtime seguiam o idioma do editor. As mensagens dos dois lados foram unificadas em `crates/protocol/src/messages`, com 11 chaves em pt-BR, en, es, ro e ru; a tabela de cada idioma é exaustiva, então um idioma incompleto não compila.
- **O protocolo ganhou um canal request/response** — `ReadMemory` ↔ `MemoryData`, correlacionados por `id` e com timeout, para a sessão não ficar presa se o plugin não responder. O restante do protocolo continua assíncrono nos dois sentidos.
- **README e documentação reescritos** — recursos, arquitetura e a página de localização atualizados para o que esta versão entrega. O README ganhou a fileira de badges (CI, CodeQL, docs, OpenSSF Scorecard, release, downloads, estrelas e licença) e uma linha de navegação para a documentação, os releases e a extensão.
- **Marcador do plugin atualizado para `PAWNPRO_DEBUG_MARKER:0.2.0`** — a extensão casa apenas o prefixo `PAWNPRO_DEBUG_MARKER`, então o reconhecimento do plugin oficial não muda; o tamanho segue 26 bytes.

### Corrigido
- Tabela de erros em [Como funciona a pausa no erro](docs/runtime-errors.md): faltavam `OP_CALL_PRI` no STACKERR e `OP_LODB_I`/`OP_STRB_I`/`OP_LIDX_B` no MEMACCESS, todos já checados pelo plugin.

### Dependências
- **SDK `rust-samp` agora vem do crates.io** (`3.4.0`), no lugar da dependência `git` + `rev` que o projeto carregava desde o início — ela existia porque o debugger precisava de API ainda não publicada. O build deixa de depender do GitHub para resolver dependências.
- Ao longo do ciclo o SDK acompanhou o que cada versão liberou: `v3.3.1` (correção), `v3.4.0`, o acessor `Amx::hlw` (exigido pelo HEAPLOW) e `AmxDbg::function_address` (exigido pelos breakpoints de função).
- Atualizações do **Dependabot** nos três ecossistemas, agrupadas por ecossistema a partir de #13: `cargo` (`serde` 1.0.228 → 1.0.229 e o grupo cargo com 2 atualizações), `github-actions` (`checkout`, `upload-artifact`, `download-artifact`, `setup-python`, `rust-cache`, `action-gh-release`) e `pip` (`mkdocs-material`, `pymdown-extensions`). Nenhuma altera comportamento do debugger.
- **Lógica genérica de VM devolvida ao SDK** (`rust-samp-sdk` 3.4.0) — a numeração dos opcodes, o tamanho das instruções, o decodificador da relocação por computed-goto (`OpcodeMap`), a caminhada da cadeia de frames e a leitura de faixas de memória eram fatos da VM AMX que viviam aqui. Agora vêm de `samp::debug::opcode`, `samp::debug::stack` e `Amx::read_bytes`/`call_stack`/`data_only`, e o plugin ficou ~250 linhas menor sem perder comportamento.
- Atualizações de `serde` (1.0.229), `serde_json` (1.0.151), das GitHub Actions e das dependências da documentação.

### Infraestrutura
- **CodeQL** migrado para advanced setup, garantindo as duas análises (`actions`, `rust`) em todo pull request, com `CODEOWNERS`.
- **OpenSSF Scorecard** — análise semanal e em push no `master`, publicando o resultado na API pública do OpenSSF e o SARIF no code scanning.
- **Dependabot** para `github-actions`, `cargo` e `pip`, agrupado em um PR por ecossistema.

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
