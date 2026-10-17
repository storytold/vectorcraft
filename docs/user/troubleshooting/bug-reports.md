---
title: Report a problem
description: What to include when you report a VectorCraft bug, and where to post it.
order: 20
verified: 0.10.0
---

Open an issue in the [issue tracker](https://github.com/storytold/vectorcraft/issues) or ask in [Discord](https://discord.gg/artcraft). Include what you did, what you expected, what happened, and the details below.

## What to include

1. The VectorCraft version. The first line of the log file names it.
2. That you use Windows 11, and your graphics processor or processors.
3. Steps that make the problem happen, and the file if the problem is with one file and you can share it.
4. The log file `vectorcraft.log` from `%APPDATA%\VectorCraft\logs`. Each start moves the previous log to `vectorcraft.1.log`, so if VectorCraft crashed, the log of that run is the next one over. Attach both files if you are not sure.
5. If VectorCraft closed by itself, the matching *Application Error* entry for `vectorcraft.exe` in **Event Viewer** › **Windows Logs** › **Application**. The name of the faulty module in it helps most.

## Before you post

- Check the [releases page](https://github.com/storytold/vectorcraft/releases) to see whether a newer version fixes it.
- Search the [issue tracker](https://github.com/storytold/vectorcraft/issues) for the same problem. Add what you know to that issue instead of opening a second one.
