# Recursos

| Recurso | Status | Detalhe |
|---------|:------:|---------|
| Breakpoints simples | :material-check: | Por linha. |
| Breakpoints condicionais | :material-check: | `var OP valor` (`==` `!=` `<` `>` `<=` `>=`); `int`/`Float:`/`bool:`/hex. |
| Hit count | :material-check: | `N`, `==N`, `>=N`, `<=N`, `>N`, `<N`, `%N`. |
| Logpoints | :material-check: | Mensagem com `{variável}` interpolada, sem pausar. |
| Step in / over / out | :material-check: | |
| Inspeção de variáveis | :material-check: | int, `Float:`, `bool:`, array, hex — em escopo. |
| Watch / hover | :material-check: | |
| Editar variável | :material-check: | Durante a pausa (`setVariable`). |
| Pausar em erro de runtime | :material-check: | Divisão por zero e índice fora do limite; pausa na linha, antes do abort. SA-MP e open.mp. Ver [Pausa no erro](runtime-errors.md). |
| Mensagens localizadas | :material-check: | pt-BR, en, es, ro, ru (segue o idioma do editor). |
| Call stack multi-frame | :material-checkbox-blank-outline: | Hoje mostra um frame; caminhar a pilha está planejado. |
| Data breakpoints | :material-checkbox-blank-outline: | Pausar quando uma variável muda — em avaliação. |
| Mais erros de runtime | :material-checkbox-blank-outline: | STACKERR / MEMACCESS / HEAPLOW — em avaliação. |

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

## Logpoints

Um breakpoint com mensagem: em vez de pausar, o plugin interpola cada `{expr}`
com o valor da variável e emite o texto no console do editor, seguindo a
execução. `{{` e `}}` são chaves escapadas.
