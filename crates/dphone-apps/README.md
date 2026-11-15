# dphone-apps

The app runtime of Open DPhone: WebAssembly components run by Wasmtime's Pulley interpreter, the host side of the app interface, app packages, and the `AppManager` the main role runs.

- `wit/app.wit` is the interface, defined once.  `crates/app-sdk` is the guest crate generated from it; the built-in apps are in `apps/`.
- An app gets only the host interfaces its manifest lists under `capabilities`.  `hotdesk` and `diagnostics` are reserved for built-in apps.
- Each app has a memory cap (4 MiB by default, 16 MiB at most via `memoryKiB`) and a time budget per event (2 s); going over either stops the app.

## Building apps

Apps are plain `cdylib` crates for `wasm32-unknown-unknown`.  The host tool `dphone-apps-build` (feature `build-tool`) turns the modules cargo produces into components and compiles them to Pulley bytecode for the phone:

```text
cd apps && cargo build --release                      # modules in <target>/wasm32-unknown-unknown/release
dphone-apps-build stage --apps apps --wasm-dir <that dir> --out <root> [--no-wasm]
```

`stage` writes `<root>/<name>/{app.json,<name>.cwasm,<name>.wasm,data files}`; `<root>` goes into the image as `/usr/share/dphone/apps`.  `--no-wasm` leaves the components out (the phone only needs them if it is built with the `compile` feature and the compiled code is stale).  `dphone-apps-build build --apps apps --out <root>` runs cargo itself, and `pack <dir> -o <file.zip>` makes an installable package.

The `.cwasm` files are tied to the Wasmtime version and to the settings in `src/engine.rs`: build them with the same `dphone-apps` source the phone binary is built from.  A mismatch fails cleanly at load.

## On the phone

| Path | Contents |
| --- | --- |
| `/usr/share/dphone/apps/<name>/` | Built-in apps |
| `/nvdata/wasmapps/<name>/` | Installed apps (beside the stock `/nvdata/jsapps`, which is left alone) |
| `/nvdata/appdata/<name>/`, `/nvdata/appdata/@shared/` | `nv`, `nvshared` |
| `/tmp/appdata/<name>/`, `/tmp/appdata/@shared/` | `tmp`, `tmpshared` |

Installed packages hold a component (`.wasm`), never compiled code, so installing needs the `compile` feature in the phone binary (Cranelift with the Pulley back end only).  Without it, built-in apps work and `InstallApp` fails with a clear error.

## Tests

`cargo test -p dphone-apps` builds the apps and a misbehaving test guest, stages them for the 64-bit interpreter, and runs them under the real manager with a fake screen and a scripted PBX.  Set `DPHONE_APPS_STAGED` to a directory staged with `--target pulley64` (apps and `tests/guests`) to skip the build step.

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

Re-run on hardware with `dphone-apps-s1 <apps root>` (feature `test-util`).
