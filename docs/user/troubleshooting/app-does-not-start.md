---
title: VectorCraft does not start
description: Fix a VectorCraft window that never appears, flashes and closes, or leaves vectorcraft.exe running in the background on Windows 11.
order: 10
verified: 0.10.0
---

Install the latest release first. If that does not help, start VectorCraft once on a different graphics processor, then set **Graphics Processor** to **Automatic**.

## Symptoms

- You start VectorCraft and no window appears, but `vectorcraft.exe` stays in Task Manager.
- The window flashes and closes with no message.
- You see a black screen, or the graphics device is lost, at start on a PC with two graphics processors.

## Fix

1. End any leftover `vectorcraft.exe` in Task Manager.
2. Install the [latest release](https://github.com/storytold/vectorcraft/releases/latest). VectorCraft 0.9.0 and newer choose the graphics processor that drives your display when **Graphics Processor** is **Automatic**. If the first window never reaches the screen within 20 seconds, they start again on the next graphics processor. They also use Direct3D 12 on Windows instead of Vulkan.
3. If it still does not start, open a new Command Prompt and start VectorCraft on the integrated graphics processor for this one run:

        set WGPU_POWER_PREF=low
        "C:\Program Files\VectorCraft\vectorcraft.exe"

   Use the path of your own `vectorcraft.exe`. The path above is the one the maintainers give for the `.msi` install. For the portable version, use the unzipped `vectorcraft.exe`. `WGPU_POWER_PREF` takes `low` or `high`. It overrides your saved choice for that launch only, and your settings still save.
4. When the window opens, choose **Edit** › **Preferences…** › **Performance**, set **Graphics Processor** to **Automatic**, and restart VectorCraft. The other choices are **Power Saving (integrated)** and **High Performance (discrete)**.
5. Close the Command Prompt. Start VectorCraft normally.

To choose a graphics processor by name instead, set `WGPU_ADAPTER_NAME` to part of its name, for example `set WGPU_ADAPTER_NAME=Intel`.

Do not use `VECTORCRAFT_NO_PREFS=1` to get past this problem. It starts VectorCraft without your preferences, but it also stops VectorCraft from saving them, and `set` keeps it for the rest of that Command Prompt window.

## Why this happens

On a laptop or a PC with two graphics processors, VectorCraft can pick one that does not drive your display. The window then never shows. Faulty graphics drivers can also stop the app before its window appears.

In 0.9.0, a stray `dxcompiler.dll` could make VectorCraft fall back to OpenGL, so it started but ran slower ([issue 1194](https://github.com/storytold/vectorcraft/issues/1194)). VectorCraft 0.10.0 compiles its Direct3D 12 shaders with the compiler that comes with Windows, so that file no longer matters.

## Still stuck?

[Report a problem](bug-reports.md) and attach `%APPDATA%\VectorCraft\logs\vectorcraft.log`. Its first lines name the graphics processors and the one VectorCraft chose. You can also ask in [Discord](https://discord.gg/artcraft).
