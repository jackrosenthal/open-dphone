# Building from source

## Prerequisites

- **An x86-64 Linux machine.**  The Nix flake defines builds for `x86_64-linux` only.
- **[Nix](https://nixos.org/download/)**, with the `nix-command` and `flakes` features enabled.  Nix provides every other tool, including the Rust toolchain, the ARM C compiler for the kernel, and Node.js for the web interface.
- **A network connection for the first build.**  Nix downloads the Linux 7.3 source and two stock firmware images from the vendor's server.  The voice DSP firmware is proprietary and is not in the repository, so the image build extracts the DSP firmware from the stock images.  Later builds reuse the downloads from the Nix store.

Clone the repository:

```sh
git clone https://github.com/jackrosenthal/open-dphone.git
cd open-dphone
```

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/dphone` | The firmware's single program: the init role, the main role, the SSH server, and the [`dphone` command](dphone-command.md). |
| `crates/dphone-early` | The early-boot stage built into the kernel image. |
| `crates/dphone-update`, `crates/dphone-ctrl` | The firmware updater, the trial boot, and `dphone ramboot`. |
| `crates/dphone-hal` | The hardware abstraction layer, with one backend for the phone and one for the simulator. |
| `crates/dphone-ui`, `crates/dphone-sip`, `crates/dphone-media`, `crates/dphone-vpm`, `crates/dphone-net`, `crates/dphone-bt`, `crates/dphone-dpma`, `crates/dphone-config`, `crates/dphone-state` | The main role's subsystems: the screens, SIP, audio and the voice DSP, the network, Bluetooth, the DPMA client, the configuration, and the state store that connects the subsystems. |
| `crates/dphone-apps`, `crates/dphone-app-sdk`, `apps/` | The app runtime, the SDK for app developers, and the built-in apps.  `apps/` is a separate Cargo workspace that targets WebAssembly. |
| `crates/dphone-web`, `web/`, `proto/` | The web server, the web frontend (TypeScript and React), and the service definitions that both are generated from. |
| `crates/dphone-image`, `crates/eff`, `crates/jffs2-image` | The host tool that builds and inspects firmware images, and the libraries for the image formats. |
| `kernel/` | The port of Linux 7.3 to the phones, as patches and added files.  `kernel/README.md` describes the kernel's constraints and how to work on the port. |
| `nix/`, `flake.nix` | The build recipes. |
| `open-dpma/` | Open DPMA, the Asterisk module, which has a separate README. |
| `xtask/` | `cargo xtask`, the developer commands that are not builds. |
| `assets/` | The fonts, icons, and artwork used by the user interface. |
| `docs/`, `zensical.toml` | This documentation site. |

## Firmware images

Each model has a flake output named `eff-<model>`, where the model is `d40`, `d45`, `d50`, `d60`, `d62`, `d65`, or `d70`.  From the root of the repository, run:

```sh
nix build .#eff-d60
```

The build leaves a `result` link in the current directory that points to the image file in the Nix store.  The file is named `<version>_<model>_firmware.eff`, e.g., `3_0_0_D60_firmware.eff`, and the version comes from the workspace version in the root `Cargo.toml`.

To print the path without creating a link:

```sh
nix build .#eff-d60 --no-link --print-out-paths
```

To build the images for all seven models into one directory, run `nix build .#all`.

!!! note "Nix builds only what git tracks"

    A flake in a git repository sees only the files that git tracks, including uncommitted changes to those files.  A new file is invisible to the build until you run `git add`.

The kernel, the root file system, and the `dphone` binary are identical for every model.  The images differ only in the device tree attached to the kernel and in the model name in the image header.  A phone rejects an image built for another model.

### Image components

Each component of an image is a separate flake output.  Building one component is faster than building a whole image.

| Output | Result |
| --- | --- |
| `dphone` | The firmware binary for the phone: static, for ARMv6. |
| `dphone-early` | The early-boot stage. |
| `kernel` | The kernel (`zImage`), a device tree for each model, and the loadable modules. |
| `kernel-<model>` | `kernel.bin`, the kernel and the model's device tree as written to a kernel partition. |
| `kernel-source` | The Linux 7.3 tree with the port applied and nothing built, for editing. |
| `apps` | The built-in apps, compiled for the phone. |
| `web` | The web frontend bundle that `dphone` embeds. |
| `brush`, `coreutils`, `sed` | The shell and utilities for the root file system. |
| `rootfs-tree`, `rootfs` | The root file system as a directory, and as the JFFS2 image that goes into the EFF image. |
| `dphone-image` | The host tool that packs and unpacks EFF images, JFFS2 images, and kernel images. |

Any change under `kernel/` rebuilds the whole kernel, because a Nix build cannot reuse object files from an earlier build.

`dphone-image` can inspect an image file without a phone.  For example, to show the model and version of an EFF image:

```sh
nix run .#dphone-image -- eff info result
```

## Cargo workspace

Nix builds the images, but Cargo is faster for the edit, build, and test cycle on the workstation.  `nix develop` opens a shell with the same Rust toolchain as the Nix build, including the targets for the phone and for WebAssembly, along with the other tools that the build scripts and the web frontend need:

```sh
nix develop
```

The root `Cargo.toml` lists the firmware's crates, `xtask`, and the Open DPMA crates.  Three of the Open DPMA crates (`dpma-sys`, `dpma-ast`, and `res_open_dpma`) read the headers of an Asterisk source tree and build only when `ASTERISK_SOURCE_DIR` is set, so exclude those three from workspace-wide commands:

```sh
cargo build --workspace --exclude dpma-sys --exclude dpma-ast --exclude res_open_dpma
```

Building a single crate needs no exclusions.  `cargo build -p dphone` builds the firmware binary for the workstation, which is the binary that the [simulator](simulator.md) runs.

The phone's binary is built for the `arm-unknown-linux-musleabi` target.  `.cargo/config.toml` sets the linker and the processor flags, so the command needs only the target:

```sh
cargo build --release -p dphone --target arm-unknown-linux-musleabi
```

[`cargo xtask deploy`](test-builds.md#binary-deploy) runs the same build and pushes the result to a phone.  The release profile optimizes the whole program as one unit, so a release build is much slower than a debug build.

### Web frontend

`dphone` embeds the web frontend from `web/dist`.  If that directory does not exist, the Cargo build embeds a placeholder page.  To build the frontend first:

```sh
cd web
pnpm install --frozen-lockfile
pnpm build
```

## Tests and checks

To run the tests for the firmware's crates on the workstation:

```sh
cargo test --workspace --exclude dpma-sys --exclude dpma-ast --exclude res_open_dpma
```

Some tests compare against stock firmware images, register with an Asterisk server, or log in with the OpenSSH client.  These tests are skipped when the workstation lacks the images, the server, or the client, so a passing `cargo test` run does not cover everything.

The built-in apps are a separate workspace.  To run the apps' unit tests on the workstation, use the alias defined in `apps/.cargo/config.toml`:

```sh
cd apps
cargo host-test
```

The flake's checks run everything, with nothing skipped.  CI runs these checks on every push.

| Command | Description |
| --- | --- |
| `nix build .#checks.x86_64-linux.tests` | Runs the tests for every crate and for the apps, with the stock images, an Asterisk server with Open DPMA, and an SSH client available.  The tests that depend on those are required to run. |
| `nix build .#checks.x86_64-linux.clippy` | Lints the workspace and the apps with Clippy.  Any warning fails the check. |
| `nix build .#checks.x86_64-linux.jffs2-mount` | Mounts the root file system image with the kernel's JFFS2 driver in a virtual machine, on simulated NAND flash and on a RAM device, and compares the files against the source directory.  Requires `/dev/kvm`. |
| `nix flake check` | Runs every check, and builds the kernel, the utilities, and the `dphone` binary. |

CI (`.github/workflows/ci.yml`) also builds the image for every model and uploads each one as a workflow artifact named `firmware-<model>`, so you can get images from a pushed branch without building locally.

The Open DPMA tests run in a container against each supported Asterisk version.  Run `cargo xtask ci` in `open-dpma/`; see `open-dpma/README.md` for details.

## xtask commands

| Command | Description |
| --- | --- |
| `cargo xtask deploy <phone>` | Builds `dphone` for the phone, pushes the binary, and restarts the main role.  See [Binary deploy](test-builds.md#binary-deploy). |
| `cargo xtask kernel-sync <linux tree>` | Regenerates `kernel/patches`, `kernel/files`, and `kernel/configs` from a Linux 7.3 checkout that contains the port.  See `kernel/README.md` for the procedure. |
| `cargo xtask tz-table` | Regenerates the phone's table of time zone rules from the workstation's zone database. |
