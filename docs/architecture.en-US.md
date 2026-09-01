# Architecture

The debugger is a Cargo workspace with three crates, plus the SDK as a
dependency.

```
editor (VS Code, DAP)
   │  Debug Adapter Protocol (stdio)
   ▼
dap-adapter  ──launches──►  SA-MP/open.mp server
   │  own protocol (NDJSON / local socket)      │ loads
   └─────────────────────────────────────────────┤
                                                  ▼
                                            debug-plugin  (inside the VM)
```

## Crates

| Crate | Kind | Role |
|-------|------|------|
| `protocol` | `lib` | Shared types for the plugin ↔ adapter IPC (commands and events as NDJSON over a local socket) and the [localized messages](i18n.md). |
| `debug-plugin` | `cdylib` | Loaded by the server. Installs the debug hook, decides when to pause (breakpoint/step/data breakpoint/error), walks the stack, collects variables, reads and writes data memory, and blocks the VM until the editor says continue. |
| `dap-adapter` | `bin` | Translates DAP ↔ the own protocol. Launches the server as a child process (which dies with it), relays breakpoints and events, and evaluates watch/console expressions. |

## Shared SDK

The parser for the `AMX_DBG` format (address ↔ line ↔ symbol ↔ function) and the
VM primitives come from the [`rust-samp`](https://rust-samp.nullsablex.com/) SDK:

- `samp::debug` / `samp_sdk::debug` — parser for the debug block.
- `Amx::cip/frame/stack/heap/hlw/stp/pri/alt` — the VM registers.
- `Amx::read_cell/write_cell` — data (inspecting and editing variables).
- `Amx::read_bytes` — raw memory ranges (the `readMemory` hex view).
- `Amx::read_code` / `Amx::opcode_map` — code and opcode decoding under
  relocation (see [Pausing on an error](runtime-errors.md)).
- `samp::debug::opcode` — opcode numbering, `STK_MARGIN` and each instruction's
  size (`operand_cells`), which the line scan uses to step forward.
- `samp::debug::stack::walk` / `Amx::call_stack` — walking the frame chain.
- `Amx::data_only` — wraps a paused VM for data-side access with no function
  table (the socket thread's situation during a pause).

Using the SDK as the single source avoids duplicating the parser between plugin
and adapter (the adapter depends on `rust-samp-sdk` with `default-features =
false, features = ["debug"]` — the pure logic only, no FFI). Both come from
crates.io at version **3.4.0**; the build has no `git` dependency.

## The flow of a pause

1. The VM calls the debug hook on every line (`.amx` compiled with `-d3`).
2. The plugin decides whether to pause (breakpoint/condition/hit-count/step/data
   breakpoint/runtime error).
3. It collects the in-scope variables and the stack frames, and sends an event to
   the adapter.
4. It **blocks** the VM (the server freezes — expected in development) until the
   editor says continue/step.

## Direction of messages

The protocol is asynchronous both ways: the adapter sends **commands**
(breakpoints, step, continue, edit a variable) and the plugin sends **events**
(pause, log, exit). Nothing waits for a reply — except one path:

`Command::ReadMemory` ↔ `Event::MemoryData` is a **request/response** pair,
correlated by a sequential `id`. The adapter registers the pending request, the
socket reader thread hands the bytes to the caller waiting for them, and a
**timeout** drops the pending entry if the plugin never answers — the session
never gets stuck.

## Compilation

- **Plugin** → the server's architecture (SA-MP/open.mp are 32-bit →
  `i686-unknown-linux-gnu`).
- **Adapter** → the host's architecture (where the editor runs).
- Edition **2024**, `resolver = "3"`.
