# How pausing on an error works

Debuggers for other languages pause on the exact line of an error (division by
zero, index out of bounds). On the AMX VM that is not trivial, and it is the
debugger's most technical feature. This page explains how it works.

## The problem

The VM aborts a runtime error through the `ABORT` macro, which **returns
immediately** from `amx_Exec` without calling the debug hook and without
preserving the exact `cip` of the instruction. In other words: when the error
happens the hook is never called — there is no way to "catch" the error
afterwards.

The way out is to **predict**: the hook is called on every line; in it, we look at
the upcoming instruction and check whether it will fail with the current
registers, pausing **before** the VM aborts.

## `OP_BREAK` is per line, not per instruction

The compiler emits an `OP_BREAK` at the **start** of each source line — not before
every instruction. The dangerous instruction (a division, an `OP_BOUNDS`) usually
sits in the **middle** of the line, after several `load`/`push`/`pop` that change
the registers.

So looking at the opcode right after the break is not enough: the debugger
**scans the line** (from the break to the next break), simulating the `pri`/`alt`
registers from the real state and reading the data segment (`Amx::read_cell`) for
operands that come from variables. When it reaches a risky instruction, it checks
it with the correct values.

## Relocation (computed goto)

On servers built with computed goto (GCC/Clang — the SA-MP and open.mp builds),
the loader **rewrites every opcode** in the code segment into the **address** of
the label that handles it. So `Amx::read_code(cip)` returns a pointer, not the
opcode number.

To recover the opcode, the debugger inverts the VM's dispatch table, obtained at
runtime through `Amx::opcode_table` (the same mechanism the loader uses). That
makes detection **portable**: it works on SA-MP and open.mp without depending on
either one's source, because both use the same AMX VM.

## Detected errors

| Error | Opcode / check | Condition (faithful to `amx.c`) |
|-------|----------------|----------------------------------|
| Division by zero | `OP_SDIV` / `OP_UDIV` | the divisor (`alt`) is zero |
| Division by zero | `OP_SDIV_ALT` / `OP_UDIV_ALT` | the divisor (`pri`) is zero |
| Index out of bounds | `OP_BOUNDS` | `(unsigned) pri > limit` |
| Stack/heap collision (`STACKERR`) | `OP_STACK` / `OP_HEAP` / `OP_PROC` / `OP_CALL` / `OP_CALL_PRI` (`CHKMARGIN`) | `hea + STKMARGIN > stk` (on `CALL`, anticipating the callee's `PROC`) |
| Heap underflow (`HEAPLOW`) | `OP_HEAP` (`CHKHEAP`) | `hea < hlw` |
| Invalid memory access (`MEMACCESS`) | `OP_LOAD_I` / `OP_LODB_I` / `OP_STOR_I` / `OP_STRB_I` / `OP_LIDX` / `OP_LIDX_B` (`VERIFYADDRESS`) | address within `[hea, stk)` or `>= stp` |

On detection the debugger pauses with `reason: "exception"` and the message in the
editor's language, showing the line and the variables — just like a regular
breakpoint.

!!! note "Conservative by design"
    The `STACKERR`/`HEAPLOW`/`MEMACCESS` checks track `stk`/`hea` along the line
    and only fire while that tracking is exact; any branch or unmodeled opcode
    (a jump, a `sysreq`, arithmetic) turns them off — **never** a false positive.
    This is not "break on any exception".

## SDK primitives used

Every read from the VM comes from the
[`rust-samp`](https://rust-samp.nullsablex.com/) SDK, with no hand-written FFI in
the debugger:

- `Amx::pri()` / `Amx::alt()` — the accumulator registers.
- `Amx::read_code(offset)` — reads the code segment (the instruction).
- `Amx::opcode_table(count)` — the dispatch table, to decode under relocation.
- `Amx::read_cell(addr)` — reads a variable's real value during the simulation.
