# Getting started

To debug you need two things: the **PawnPro** extension in your editor and the
debugger **plugin** in your server. The extension handles everything when a
session starts (recompiling, launching the server, connecting) — the only manual
step is putting the plugin in the server, **once**.

## 1. Download the plugin

Grab the plugin binary from the [releases page][releases]:

- **Linux:** `pawnpro_debug.so`
- **Windows:** `pawnpro_debug.dll`

[releases]: https://github.com/NullSablex/PawnPro-Debugger/releases

## 2. Put it in the server

Copy the file into the right folder of your server:

- **SA-MP:** `plugins/pawnpro_debug.so`, and add `pawnpro_debug` to the `plugins`
  line of `server.cfg`.
- **open.mp:** `components/pawnpro_debug.so`.

!!! warning "Do not rename the file"
    The name must be **`pawnpro_debug`** (`.so` on Linux, `.dll` on Windows). The
    extension looks for the plugin by exactly that name; under any other name,
    debugging will not start.

!!! note "The extension only checks"
    On startup the extension verifies that the plugin is present and warns if it
    is missing — it does **not** install anything into your server.

## 3. Debug

Open your gamemode's `.pwn` in the editor (with the PawnPro extension) and press
**F5**. Set breakpoints in the gutter and debug as usual. The extension
recompiles the gamemode with `-d3` (the debug block, carrying lines and symbols)
before launching the server — without it there are no breakpoints and no
inspection.

While execution is paused the server is **frozen**: expected on a local
development server.

### Example `launch.json`

```json
{
  "type": "pawn",
  "request": "launch",
  "name": "Debug gamemode",
  "program": "${workspaceFolder}/gamemodes/mygm.amx"
}
```

## Next step

See [Features](features.md) for what you can do while paused — call stack,
inspecting and editing variables, data breakpoints, the memory hex view and
automatic pausing on [runtime errors](runtime-errors.md).
