# dphone-apps

The app runtime of Open DPhone: WebAssembly components run by Wasmtime's Pulley interpreter, the host side of the app interface, app packages, and the `AppManager` the main role runs.

- The interface is defined once, in `crates/dphone-app-sdk/wit/app.wit`.  `dphone-app-sdk` is the guest crate generated from it, published to crates.io for app makers (its README tells how to write an app); this crate generates the host side from the same file.  The built-in apps are in `apps/`.
- An app is one file: a WebAssembly component with its manifest in it (the custom section `dphone-app`, written by the SDK's `export_app!`; `crates/cargo-dphone-app` reads it, here and in the tool app makers build with).  Its data is in its code (`include_bytes!`); nothing is unpacked on the phone.
- An app gets the host interfaces its component imports, and no others: capabilities are not declared.  `hotdesk` and `diagnostics` are reserved for built-in apps: an app that imports one is not installed.
- Each app has a memory cap (4 MiB by default, 16 MiB at most via `memory_kib`) and a time budget per event (2 s); going over either stops the app.

## Building apps

Apps are plain `cdylib` crates for `wasm32-unknown-unknown`.  `cargo dphone-app` (`crates/cargo-dphone-app`, published with the SDK) builds one into the file the phone installs.  For the firmware's own apps, the host tool `dphone-apps-build` (feature `build-tool`) makes the same component with the same code and compiles it to Pulley bytecode for the phone:

```text
cd apps && cargo build --release                      # modules in <target>/wasm32-unknown-unknown/release
dphone-apps-build stage --wasm-dir <that dir> --out <root> [--no-wasm]
```

`stage` writes `<root>/<name>/{app.pb,<name>.cwasm,<name>.wasm}` for every module that is an app; `<root>` goes into the image as `/usr/share/dphone/apps`.  `--no-wasm` leaves the components out (the image has compiled code only).  `dphone-apps-build build --apps apps --out <root>` runs cargo itself.

The `.cwasm` files are tied to the Wasmtime version and to the settings in `src/engine.rs`: build them with the same `dphone-apps` source the phone binary is built from.  A mismatch fails cleanly at load.

## On the phone

| Path | Contents |
| --- | --- |
| `/usr/share/dphone/apps/<name>/` | Built-in apps: `app.pb`, `<name>.cwasm` |
| `/nvdata/wasmapps/<name>/` | Installed apps (beside the stock `/nvdata/jsapps`, which is left alone): `app.pb`, `<name>.wasm`, `.code.cwasm` |
| `/nvdata/appdata/<name>/`, `/nvdata/appdata/@shared/` | `nv`, `nvshared` |
| `/tmp/appdata/<name>/`, `/tmp/appdata/@shared/` | `tmp`, `tmpshared` |

`app.pb` is the phone's record of an app: its manifest (the message `dphone.app.v1.Manifest` of `crates/dphone-app-sdk/proto/manifest.proto`, icons included) with the interfaces its component imports.  The launcher's icon comes from there: the one for the phone's model if the manifest has one, else the app's, else one of the phone's own by the name the manifest gives.  `<name>.wasm` is the component as it was installed.

## Installing

An app file holds a component, never compiled code: Wasmtime trusts the compiled code it loads, so the phone compiles every installed app itself (Cranelift with the Pulley back end; the feature `compile`, which the phone binary must have: `crates/dphone` does not build without it) and keeps the result in `.code.cwasm`.  Before that it reads the manifest from the file and refuses what is not an app, an app with a built-in app's name, and a stock JavaScript package; after it, an app that imports an interface reserved for built-in apps.

Measured on a D60 (ARM1176, 2026-11-15; the web interface's "Install App", a dial tone on the handset meanwhile):

| App | Component | Compile time | Main role's memory, peak over before | Audio ticks a second during |
| --- | --- | --- | --- | --- |
| The SDK README's "hello" | 24 KiB | 5.2 s | 3.7 MB | 100 |
| Status | 50 KiB | 7.9 s | 2.3 MB | 100 |
| Light Up | 119 KiB | 16.4 s | 6.2 MB | 100 |

So a compilation takes about 2 s and 0.12 s for every KiB, far too long for the manager's thread (the running apps would stand still, and the liveness check would report the subsystem dead).  A worker thread (`src/prepare.rs`) does it, at the lowest priority (nice 19; the audio thread is `SCHED_FIFO`), one app at a time; the command `InstallApp` is answered when it is done, and the web interface waits up to ten minutes.  An app file may be 8 MiB with 1 MiB of code in it (`MAX_COMPONENT`; the code sections, since the compiler only copies data): about three minutes by these figures (the phone's own binary under `qemu-arm` takes 11 times as long for a 1,011 KiB component as for Light Up; such a component has not been timed on a phone).

After a firmware update that changes the engine, the compiled code of the installed apps no longer loads.  The manager finds that when it starts and has the worker compile each such app again; until an app's turn is over, opening it fails with a message that says so.

The cost of the compiler in the phone binary: 12.99 MB without it, 17.65 MB with it; the D60 image's root file system (JFFS2) grows from 18.4 MB to 20.9 MB of the 45 MiB partition.

## Tests

`cargo test -p dphone-apps` builds the apps and a misbehaving test guest, stages them for the 64-bit interpreter, and runs them under the real manager with a fake screen and a scripted PBX.  Set `DPHONE_APPS_STAGED` to a directory staged with `--target pulley64` (apps and `tests/guests`) to skip the build step.

The apps' own tests (the unit tests of each app crate, and the tests of a game's library, which lies in the app's directory, e.g., `apps/lightup/game`) are in the `apps/` workspace, which `cargo test` in the firmware workspace does not reach.  Run them with `cargo host-test` in `apps/`: an alias for `cargo test --target host-tuple`, because that workspace otherwise builds for WebAssembly.  The Nix check `tests` runs both.

## Spike S1 (measured 2026-11-13)

Wasmtime 50 with only the `runtime`, `component-model`, and `std` features builds for `arm-unknown-linux-musleabi` with no C: its build script compiles `helpers.c` only on hosts that have a native Cranelift back end.

Sizes are stripped ARMv6 binaries at `opt-level = "s"` with LTO; times are under `qemu-arm` on a workstation, so they compare engines but do not predict the phone.

| | Wasmtime/Pulley | wasmi 2.0 |
| --- | --- | --- |
| Engine in a minimal program (core modules only) | 0.89 MB | 1.30 MB |
| Whole `dphone-apps-s1` (runtime, component model, manager, fakes) | 1.64 MB | not built |
| The same with the on-device compiler (`compile`) | 5.62 MB | compiler is included |
| Host call, bare `i32 -> i32` | 0.9 to 1.2 µs | 0.22 to 0.24 µs |
| `fib(27)` | 172 to 252 ms | 94 to 114 ms |
| Component model | Yes | No (hand-written ABI needed) |

With the real apps: loading, instantiating, and starting the precompiled Status app takes 32 to 37 ms; rebuilding a screen (about 12 host calls with strings and lists) takes about 0.2 ms; compiling the 51 KiB Status component on the device takes 0.73 s.  One instance adds about 1.8 MiB of resident memory, measured natively with the 64-bit interpreter; the `qemu-arm` figure (5 to 7 MiB) includes the emulator's own translation cache and is not meaningful.

Decision: Wasmtime with Pulley, as the plan prefers; it meets the no-C constraint and the size is acceptable.  wasmi interprets about twice as fast in this test and needs no separate compiler for installed apps, so it remains the fallback if phone measurements disappoint.  Pulley is markedly slower at `opt-level = "s"`; the phone build should set `opt-level = 3` for the `pulley-interpreter` package (the better figures above).

Re-run on hardware with `dphone-apps-s1 <apps root>` (features `test-util` and `cli`).  The phone's own figures for compiling are under "Installing" above.
