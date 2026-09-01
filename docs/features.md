# Recursos

| Recurso | Status | Detalhe |
|---------|:------:|---------|
| Breakpoints simples | :material-check: | Por linha. |
| Breakpoints condicionais | :material-check: | `var OP valor` (`==` `!=` `<` `>` `<=` `>=`); `int`/`Float:`/`bool:`/hex. |
| Hit count | :material-check: | `N`, `==N`, `>=N`, `<=N`, `>N`, `<N`, `%N`. |
| Breakpoints de função | :material-check: | Parar ao entrar numa função pelo nome. |
| Logpoints | :material-check: | Mensagem com `{expressão}` interpolada, sem pausar. |
| Step in / over / out | :material-check: | |
| Call stack multi-frame | :material-check: | Caminha a cadeia de frames (FRM→retorno); função, linha e variáveis por frame. |
| Inspeção de variáveis | :material-check: | `int`, `Float:`, `bool:`, hex; arrays **expansíveis** (elementos como filhos) e arrays de char como **string** — em escopo. |
| Watch / hover | :material-check: | Expressões simples: aritmética, comparação, variáveis e `arr[i]`. |
| Autocomplete | :material-check: | Sugere variáveis em escopo no watch e no console. |
| Editar variável | :material-check: | Durante a pausa, por painel (`setVariable`) ou por expressão (`setExpression`), inclusive elementos de array (`arr[i]`). |
| Data breakpoints | :material-check: | Pausar quando um valor muda: globais, locais e **elementos de array**; watches de locais expiram ao retornar o frame. |
| Ler memória | :material-check: | Hex view da memória de dados crua a partir de qualquer variável. |
| Pausar em erro de runtime | :material-check: | Divisão por zero, índice fora do limite, colisão pilha/heap, underflow de heap e acesso inválido à memória; pausa na linha, antes do abort. SA-MP e open.mp. Ver [Pausa no erro](runtime-errors.md). |
| Mensagens localizadas | :material-check: | pt-BR, en, es, ro, ru (segue o idioma do editor). Ver [i18n](i18n.md). |

## Breakpoints condicionais

Uma condição é `variável OPERADOR valor`, avaliada no plugin contra as variáveis
em escopo. O valor pode ser inteiro (`100`, `0x64`), `Float:` (`96.5`) ou `bool:`
(`true`/`false`). A comparação promove `int`↔`Float`; `bool` só compara igualdade.

Por design é **conservador**: se a expressão não for reconhecida ou um operando
não resolver, o breakpoint **para** (melhor parar a mais do que engolir).

## Hit count

Aceita um número puro (`5` = a partir do 5º acerto), operadores (`==N`, `>=N`,
`<=N`, `>N`, `<N`) ou módulo (`%N` = a cada N acertos). A condição lógica, quando
existe, filtra **antes** de o acerto contar.

## Breakpoints de função

Em vez de uma linha, o alvo é o **nome da função**: o adaptador resolve o nome no
bloco `AMX_DBG` e envia ao plugin o endereço de entrada. Nome que não resolve
volta como não verificado ao editor, sem quebrar a sessão. Os breakpoints de linha
e de função são enviados ao plugin como um **conjunto único**.

## Logpoints

Um breakpoint com mensagem: em vez de pausar, o plugin interpola cada `{expr}`
com o valor da variável e emite o texto no console do editor, seguindo a
execução. `{{` e `}}` são chaves escapadas.

## Watch, hover e edição por expressão

O avaliador aceita um operando ou `A OP B` (**um** operador de topo, sem cadeias
com precedência). Operandos: literais (`10`, `0x0a`, `1.5`, `true`), variáveis em
escopo e elementos de array `arr[i]` — o índice pode ser literal, variável ou uma
subexpressão. Operadores: `+ - * / %` (inteiros seguem a semântica do Pawn, que
trunca) e `== != < > <= >=`.

O que não casar devolve "não avaliável", em vez de um valor inventado.

Editar aceita as duas vias do DAP: `setVariable` (dar um valor novo no painel
**Variáveis**) e `setExpression` (`arr[i] = 10` no watch ou no console). No
`setExpression` o lado esquerdo é um **lvalue** — `nome` ou `nome[expr]`; o array
inteiro não é editável, só os elementos.

## Data breakpoints

"Break on Value Change" no painel **Variáveis**: o plugin guarda o último valor
observado e pausa quando ele muda. Cobre:

- **globais** — endereço absoluto, o watch nunca expira;
- **locais** — endereço relativo ao frame; o watch **expira** quando o frame dono
  retorna (não dispara com o lixo de outro frame no mesmo slot);
- **elementos de array** (`arr[3]`) — observados individualmente.

Se o endereço não puder ser lido, o plugin é conservador: não inventa mudança.

## Ler memória

Toda variável exposta ao editor carrega um `memoryReference`, então o **hex view**
abre a partir dela e navega pela memória de dados crua (a região a partir do
endereço da variável ou do elemento, mais o `offset` pedido). É o único caminho do
protocolo que faz **request/response** com o plugin: o adaptador correlaciona o
pedido pela resposta e desiste no timeout, em vez de bloquear a sessão.

A leitura respeita o alinhamento de células de 4 bytes da VM e devolve apenas o
que conseguiu ler — no fim do segmento, menos bytes do que o pedido.
