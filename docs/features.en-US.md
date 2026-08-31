# Features

| Feature | Status | Detail |
|---------|:------:|--------|
| Plain breakpoints | :material-check: | By line. |
| Conditional breakpoints | :material-check: | `var OP value` (`==` `!=` `<` `>` `<=` `>=`); `int`/`Float:`/`bool:`/hex. |
| Hit count | :material-check: | `N`, `==N`, `>=N`, `<=N`, `>N`, `<N`, `%N`. |
| Function breakpoints | :material-check: | Stop on entry to a function by name. |
| Logpoints | :material-check: | Message with `{expression}` interpolated, without pausing. |
| Step in / over / out | :material-check: | |
| Multi-frame call stack | :material-check: | Walks the frame chain (FRM→return address); function, line and variables per frame. |
| Variable inspection | :material-check: | `int`, `Float:`, `bool:`, hex; arrays are **expandable** (elements as children) and char arrays show as a **string** — in scope. |
| Watch / hover | :material-check: | Simple expressions: arithmetic, comparison, variables and `arr[i]`. |
| Autocomplete | :material-check: | Suggests in-scope variables in the watch panel and the console. |
| Editing variables | :material-check: | While paused, from the panel (`setVariable`) or by expression (`setExpression`), array elements (`arr[i]`) included. |
| Data breakpoints | :material-check: | Pause when a value changes: globals, locals and **array elements**; watches on locals expire when the owning frame returns. |
| Reading memory | :material-check: | Hex view of raw data memory starting from any variable. |
| Pausing on a runtime error | :material-check: | Division by zero, index out of bounds, stack/heap collision, heap underflow and invalid memory access; pauses on the line, before the abort. SA-MP and open.mp. See [Pausing on an error](runtime-errors.md). |
| Localized messages | :material-check: | pt-BR, en, es, ro, ru (follows the editor's language). See [i18n](i18n.md). |

## Conditional breakpoints

A condition is `variable OPERATOR value`, evaluated in the plugin against the
variables in scope. The value can be an integer (`100`, `0x64`), a `Float:`
(`96.5`) or a `bool:` (`true`/`false`). Comparison promotes `int`↔`Float`; `bool`
only compares for equality.

It is **conservative** by design: if the expression is not recognized or an
operand does not resolve, the breakpoint **stops** anyway (better to stop too
often than to swallow a breakpoint).

## Hit count

Accepts a bare number (`5` = from the 5th hit on), operators (`==N`, `>=N`,
`<=N`, `>N`, `<N`) or modulo (`%N` = every N hits). The logical condition, when
present, filters **before** the hit is counted.

## Function breakpoints

Instead of a line, the target is the **function name**: the adapter resolves the
name in the `AMX_DBG` block and sends the entry address to the plugin. A name
that does not resolve comes back to the editor as unverified, without breaking
the session. Line and function breakpoints are sent to the plugin as a **single
set**.

## Logpoints

A breakpoint with a message: instead of pausing, the plugin interpolates every
`{expr}` with the variable's value and emits the text to the editor console,
following execution. `{{` and `}}` are escaped braces.

## Watch, hover and editing by expression

The evaluator accepts a single operand or `A OP B` (**one** top-level operator,
no chains with precedence). Operands: literals (`10`, `0x0a`, `1.5`, `true`),
in-scope variables and array elements `arr[i]` — the index can be a literal, a
variable or a subexpression. Operators: `+ - * / %` (integers follow Pawn's
truncating semantics) and `== != < > <= >=`.

Anything that does not match returns "not evaluable" instead of an invented
value.

Editing supports both DAP routes: `setVariable` (typing a new value in the
**Variables** panel) and `setExpression` (`arr[i] = 10` in the watch panel or the
console). In `setExpression` the left-hand side is an **lvalue** — `name` or
`name[expr]`; a whole array is not editable, only its elements.

## Data breakpoints

"Break on Value Change" in the **Variables** panel: the plugin keeps the last
observed value and pauses when it changes. It covers:

- **globals** — absolute address, the watch never expires;
- **locals** — frame-relative address; the watch **expires** when the owning
  frame returns (so it will not fire on another frame's leftovers in the same
  slot);
- **array elements** (`arr[3]`) — watched individually.

If the address cannot be read, the plugin stays conservative: it does not invent
a change.

## Reading memory

Every variable exposed to the editor carries a `memoryReference`, so the **hex
view** opens from it and browses raw data memory (the region starting at the
variable's or element's address, plus the requested `offset`). It is the only
path in the protocol that does **request/response** with the plugin: the adapter
correlates the request with its reply and gives up on a timeout instead of
blocking the session.

Reads honor the VM's 4-byte cell alignment and return only what could actually be
read — at the end of the segment, fewer bytes than requested.
