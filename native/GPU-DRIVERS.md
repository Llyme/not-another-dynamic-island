# Graphics drivers: what to know

NADI draws with Direct3D 11, Direct2D and DirectComposition, and it keeps running for days. That makes it unusually exposed to bugs
in a GPU vendor's user-mode driver, because that driver is a DLL loaded into NADI's own process: when it crashes, NADI crashes with
it, whatever NADI was doing. This page records what has been seen with AMD's driver and what NADI does about it, so that nobody has
to rediscover it.

## What AMD's driver does

- **It crashes in-process, on a thread of its own.** The crashes seen were access violations (`0xc0000005`) inside `amdxx64.dll`,
  AMD's user-mode Direct3D driver, on one of the driver's own worker threads, not on a NADI thread. NADI's code is not on the stack;
  there is nothing to catch, and nothing NADI can do to recover once it happens.
- **It is not specific to NADI.** The same crash, in the same module, has been seen taking down other programs (games, and Windows
  Explorer) on the same machines, with NADI not running. Treat it as a driver bug that NADI is one more victim of.
- **Updating the driver did not make it go away.** It was seen on more than one Adrenalin release, including a fresh install with
  the old one removed first. Do not tell people "update your driver" as if it were the fix.
- **The timing pointed at NADI's direct Direct3D path.** The first crashes showed up shortly after `liquidgpu.rs` was added: the
  pixel shader that draws the "liquid" edges straight through `ID3D11DeviceContext` (everything else goes through Direct2D). This is
  a correlation, not a proof; an A/B build that draws the liquid the old way (Direct2D only) was never run to confirm it.
- **AMD injects a hook into every process.** `amdihk64.dll` (AMD's hotkey and overlay hook) is loaded into NADI, as into every app,
  whether or not its features are turned on in Adrenalin. It is not known to be the cause, but it means AMD code runs inside NADI
  even when NADI is not drawing.
- **Global driver features touch every app.** Adrenalin's features that apply to the global profile (HYPR-RX, Fluid Motion Frames,
  Super Resolution, Anti-Lag, Boost, Chill) hook every program that draws, NADI included. When chasing a crash, turning them off
  globally is a cheap thing to try.

## What NADI does about it

- **Software Drawing** (Settings > Performance, `config.software_draw`). NADI's devices are made on WARP, Windows' own software
  rasterizer, instead of the GPU (`gfx::set_software`, read when `Gfx::new` runs). With it on, no AMD DLL loads into NADI at all, so
  a driver crash cannot take NADI down. It costs a lot of processor time while things move (well over one core with the liquid
  animating), so it is a safety switch, not a default. It switches live: `Frame::rebuild_device`.
- **Device rebuilds without leaks.** When the device is lost, or drawing switches to or from software, `Frame::rebuild_device` makes
  new devices and drops every piece made with the old ones (screens, launcher, modal, banner, tip, menus, the liquid's shaders, the
  image and icon caches). Before the old devices are dropped, the old DirectComposition device must be committed and waited on
  (`Commit` then `WaitForCommitCompletion`): until it commits, DComp keeps the visuals it last showed, and every Direct3D and Direct2D
  object with them. Without that, each rebuild leaked about 25 to 30 MB.
- **A crash recorder** (`crashdump.rs`). An unhandled exception writes one line to `%APPDATA%\NADI\crash.log` (the faulting module
  and offset, and what NADI was doing, from `crashdump::doing`) and a small minidump beside it, `crash-<time>.dmp`, keeping the newest
  three. A line naming `amdxx64.dll` (or another vendor's driver: `nvwgf2umx.dll`, `igd10iumd64.dll`, ...) is a driver crash, not a
  NADI bug.

## When you change the drawing code

- Prefer Direct2D. Every direct `ID3D11DeviceContext` path (shaders, render targets, copies) is more driver surface, and the one
  NADI has is the prime suspect above. If you add one, keep a Direct2D or software fallback.
- Test with Software Drawing on as well as off. WARP is slower but correct, and it is what people who hit driver crashes will run.
- Assume the device can be lost at any time, and that a rebuild must free everything the old device made.
- A crash report with a driver module in it: ask for the `crash.log` line and the newest dump, and for the driver version. Suggest
  Software Drawing as the workaround, and turning off the driver's global features as the next thing to try.
