# Arquitetura

O debugger é um workspace Cargo com três crates, mais o SDK como dependência.

```
editor (VS Code, DAP)
   │  Debug Adapter Protocol (stdio)
   ▼
dap-adapter  ──lança──►  servidor SA-MP/open.mp
   │  protocolo próprio (NDJSON / socket local)   │ carrega
   └──────────────────────────────────────────────┤
                                                   ▼
                                             debug-plugin  (dentro da VM)
```

## Crates

| Crate | Tipo | Papel |
|-------|------|-------|
| `protocol` | `lib` | Tipos compartilhados do IPC plugin ↔ adaptador (comandos e eventos em NDJSON sobre socket local) e as [mensagens localizadas](i18n.md). |
| `debug-plugin` | `cdylib` | Carregado pelo servidor. Instala o debug hook, decide pausar (breakpoint/step/data breakpoint/erro), caminha a pilha, coleta variáveis, lê e escreve a memória de dados, e bloqueia a VM até o editor mandar continuar. |
| `dap-adapter` | `bin` | Traduz DAP ↔ protocolo próprio. Lança o servidor como processo filho (morre junto), repassa breakpoints e eventos, e avalia as expressões do watch/console. |

## SDK compartilhado

O parser do formato `AMX_DBG` (endereço ↔ linha ↔ símbolo ↔ função) e as
primitivas de VM vêm do SDK [`rust-samp`](https://rust-samp.nullsablex.com/):

- `samp::debug` / `samp_sdk::debug` — parser do bloco de debug.
- `Amx::cip/frame/stack/heap/hlw/stp/pri/alt` — registradores da VM.
- `Amx::read_cell/write_cell` — dados (inspecionar/editar variáveis).
- `Amx::read_bytes` — faixa de memória crua (o hex view do `readMemory`).
- `Amx::read_code` / `Amx::opcode_map` — código e a decodificação de opcodes sob
  relocação (ver [Pausa no erro](runtime-errors.md)).
- `samp::debug::opcode` — numeração dos opcodes, `STK_MARGIN` e o tamanho de
  cada instrução (`operand_cells`), que a varredura da linha usa para avançar.
- `samp::debug::stack::walk` / `Amx::call_stack` — caminhada da cadeia de frames.
- `Amx::data_only` — embrulha a VM pausada para acesso só de dados, sem tabela
  de funções (é a situação da thread do socket durante a pausa).

Usar o SDK como fonte única evita duplicar o parser entre o plugin e o adaptador
(o adaptador depende do `rust-samp-sdk` com `default-features = false, features =
["debug"]`, só a lógica pura, sem FFI).

## Fluxo de uma pausa

1. A VM chama o debug hook a cada linha (`.amx` compilado com `-d3`).
2. O plugin decide pausar (breakpoint/condição/hit-count/step/data breakpoint/erro
   de runtime).
3. Coleta as variáveis em escopo e os frames da pilha, e envia um evento ao
   adaptador.
4. **Bloqueia** a VM (o servidor congela — esperado em dev) até o editor mandar
   continuar/step.

## Direção das mensagens

O protocolo é assíncrono nos dois sentidos: o adaptador manda **comandos**
(breakpoints, step, continuar, editar variável) e o plugin manda **eventos**
(pausa, log, saída). Nada espera resposta — exceto uma via:

`Command::ReadMemory` ↔ `Event::MemoryData` é um par **request/response**,
correlacionado por um `id` sequencial. O adaptador registra o pedido pendente, a
thread leitora do socket entrega os bytes ao chamador que espera, e um **timeout**
descarta o pendente se o plugin não responder — a sessão nunca fica presa.

## Compilação

- **Plugin** → arquitetura do servidor (SA-MP/open.mp são 32-bit →
  `i686-unknown-linux-gnu`).
- **Adaptador** → arquitetura do host (onde o editor roda).
- Edição **2024**, `resolver = "3"`.
