# Como funciona a pausa no erro

Depuradores de outras linguagens pausam na linha exata de um erro (divisão por
zero, índice fora do array). Na VM AMX isso não é trivial, e é o recurso mais
técnico do debugger. Esta página explica como ele funciona.

## O problema

A VM aborta um erro de runtime com a macro `ABORT`, que **retorna imediatamente**
de `amx_Exec` sem chamar o debug hook e sem preservar o `cip` exato da
instrução. Ou seja: quando o erro acontece, o hook nunca é chamado — não dá para
"pegar" o erro depois.

A saída é **prever**: o hook é chamado a cada linha; nele, olhamos a próxima
instrução e checamos se ela vai falhar com os registradores atuais, pausando
**antes** de a VM abortar.

## O `OP_BREAK` é por linha, não por instrução

O compilador emite um `OP_BREAK` no **início** de cada linha-fonte — não antes de
cada instrução. A instrução perigosa (uma divisão, um `OP_BOUNDS`) costuma estar
no **meio** da linha, depois de vários `load`/`push`/`pop` que alteram os
registradores.

Por isso não basta olhar o opcode logo após o break: o debugger **varre a linha**
(do break até o próximo break), simulando os registradores `pri`/`alt` a partir do
estado real, e lendo o segmento de dados (`Amx::read_cell`) para os operandos que
vêm de variáveis. Quando chega numa instrução de risco, checa com os valores
corretos.

## A relocação (computed-goto)

Em servidores compilados com computed-goto (GCC/Clang — os builds de SA-MP e
open.mp), o loader **reescreve cada opcode** no segmento de código para o
**endereço** do label que o trata. Então `Amx::read_code(cip)` devolve um
ponteiro, não o número do opcode.

Para recuperar o opcode, o debugger inverte a tabela de despacho da VM,
obtida em runtime por `Amx::opcode_table` (o mesmo mecanismo que o loader usa).
Isso torna a detecção **portável**: funciona em SA-MP e open.mp, sem depender do
código-fonte de nenhum dos dois, porque ambos usam a mesma VM AMX.

## Erros detectados

| Erro | Opcode | Condição |
|------|--------|----------|
| Divisão por zero | `OP_SDIV` / `OP_UDIV` | divisor (`alt`) é zero |
| Divisão por zero | `OP_SDIV_ALT` / `OP_UDIV_ALT` | divisor (`pri`) é zero |
| Índice fora do limite | `OP_BOUNDS` | `(unsigned) pri > limite` |

Ao detectar, o debugger pausa com `reason: "exception"` e a mensagem no idioma do
editor, mostrando a linha e as variáveis — como um breakpoint normal.

!!! note "Cobertura parcial por design"
    Só erros previsíveis por análise da próxima instrução. `STACKERR`,
    `MEMACCESS` e `HEAPLOW` dependem de estado dinâmico e estão em avaliação — não
    é "pausa em qualquer exceção".

## Primitivas do SDK usadas

Toda a leitura da VM vem do SDK [`rust-samp`](https://rust-samp.nullsablex.com/),
sem FFI manual no debugger:

- `Amx::pri()` / `Amx::alt()` — registradores acumuladores.
- `Amx::read_code(offset)` — lê o segmento de código (a instrução).
- `Amx::opcode_table(count)` — a tabela de despacho, para decodificar sob
  relocação.
- `Amx::read_cell(addr)` — lê o valor real de uma variável durante a simulação.
