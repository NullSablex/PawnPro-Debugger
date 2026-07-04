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
| `protocol` | `lib` | Tipos compartilhados do IPC plugin ↔ adaptador (comandos e eventos em NDJSON sobre socket local). |
| `debug-plugin` | `cdylib` | Carregado pelo servidor. Instala o debug hook, decide pausar (breakpoint/step/erro), coleta variáveis e bloqueia a VM até o editor mandar continuar. |
| `dap-adapter` | `bin` | Traduz DAP ↔ protocolo próprio. Lança o servidor como processo filho (morre junto), repassa breakpoints e eventos. |

## SDK compartilhado

O parser do formato `AMX_DBG` (endereço ↔ linha ↔ símbolo ↔ função) e as
primitivas de VM vêm do SDK [`rust-samp`](https://rust-samp.nullsablex.com/):

- `samp::debug` / `samp_sdk::debug` — parser do bloco de debug.
- `Amx::cip/frame/stack/heap/stp/pri/alt` — registradores da VM.
- `Amx::read_cell/write_cell` — dados (inspecionar/editar variáveis).
- `Amx::read_code` / `Amx::opcode_table` — código (decodificar opcodes; ver
  [Pausa no erro](runtime-errors.md)).

Usar o SDK como fonte única evita duplicar o parser entre o plugin e o adaptador
(o adaptador depende do `rust-samp-sdk` com `default-features = false, features =
["debug"]`, só a lógica pura, sem FFI).

## Fluxo de uma pausa

1. A VM chama o debug hook a cada linha (`.amx` compilado com `-d3`).
2. O plugin decide pausar (breakpoint/condição/hit-count/step/erro de runtime).
3. Coleta as variáveis em escopo e envia um evento ao adaptador.
4. **Bloqueia** a VM (o servidor congela — esperado em dev) até o editor mandar
   continuar/step.

## Compilação

- **Plugin** → arquitetura do servidor (SA-MP/open.mp são 32-bit →
  `i686-unknown-linux-gnu`).
- **Adaptador** → arquitetura do host (onde o editor roda).
- Edição **2024**, `resolver = "3"`.
