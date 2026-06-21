//! Plugin do servidor (Componente 2 do debugger) — `cdylib` carregado pelo
//! SA-MP/open.mp via ABI C. Instala o debug hook (`SetDebugHook`), pausa em
//! breakpoint, faz step e inspeciona memória da AMX VM (`get_ref`, com limites).
//!
//! Depende de `amxdbg` (mapeamento endereço↔linha↔símbolo) e do `samp-sdk`.
//!
//! Esqueleto — sem implementação ainda.
