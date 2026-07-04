# Começando

Para depurar você precisa de duas coisas: a extensão **PawnPro** no editor e o
**plugin** do debugger no seu servidor. A extensão cuida de tudo ao iniciar a
sessão (recompilar, subir o servidor, conectar) — a única ação manual é colocar o
plugin no servidor, **uma vez**.

## 1. Baixar o plugin

Pegue o binário do plugin na [página de releases][releases]:

- **Linux:** `pawnpro_debug.so`
- **Windows:** `pawnpro_debug.dll`

[releases]: https://github.com/NullSablex/PawnPro-Debugger/releases

## 2. Colocar no servidor

Copie o arquivo para a pasta correta do seu servidor:

- **SA-MP:** `plugins/pawnpro_debug.so` e adicione `pawnpro_debug` à linha
  `plugins` do `server.cfg`.
- **open.mp:** `components/pawnpro_debug.so`.

!!! warning "Não renomeie o arquivo"
    O nome tem de ser **`pawnpro_debug`** (`.so` no Linux, `.dll` no Windows). A
    extensão procura o plugin exatamente por esse nome; com outro nome, a
    depuração não inicia.

!!! note "A extensão só verifica"
    Ao iniciar, a extensão confere se o plugin está presente e avisa se faltar —
    ela **não instala** nada no servidor.

## 3. Depurar

Abra o `.pwn` do gamemode no editor (com a extensão PawnPro) e pressione **F5**.
Coloque breakpoints na margem e depure normalmente.

### Exemplo de `launch.json`

```json
{
  "type": "pawn",
  "request": "launch",
  "name": "Depurar gamemode",
  "program": "${workspaceFolder}/gamemodes/meugm.amx"
}
```
