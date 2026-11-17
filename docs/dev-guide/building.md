# Building from source

## Prerequisites

- **An x86-64 Linux machine.**  The Nix flake defines builds for `x86_64-linux` only.
- **[Nix](https://nixos.org/download/)**, with the `nix-command` and `flakes` features enabled.  Nix provides every other tool, including the Rust toolchain, the ARM C compiler for the kernel, and Node.js for the web interface.
- **A network connection for the first build.**  Nix downloads the Linux source and two stock firmware images from the vendor's server.  The voice DSP firmware is proprietary and not in the repository, so the image build extracts the DSP firmware from the stock images.

```sh
git clone https://github.com/jackrosenthal/open-dphone.git
cd open-dphone
```

## Firmware images

Each model has a flake output `eff-<model>`, where the model is `d40`, `d45`, `d50`, `d60`, `d62`, `d65`, or `d70`.  From the root of the repository:

```sh
nix build .#eff-d60
```

The `result` link points to the image in the Nix store, named `<version>_<model>_firmware.eff`, e.g., `3_0_0_D60_firmware.eff`.  The version is the workspace version in the root `Cargo.toml`.  `nix build .#all` builds the images of all seven models into one directory.

!!! note "Nix builds only what git tracks"

    A flake in a git repository sees only the files that git tracks, including uncommitted changes to those files.  A new file is invisible to the build until `git add`.

The kernel, the root file system, and the `dphone` binary are the same for every model.  The images differ only in the device tree attached to the kernel and in the model name in the image header.

### Image components

Each component of an image is a separate flake output.

| Output | Result |
| --- | --- |
| `dphone` | The firmware binary for the phone: static, for ARMv6. |
| `dphone-early` | The early-boot stage. |
| `kernel` | The kernel (`zImage`), a device tree for each model, and the loadable modules. |
| `kernel-<model>` | `kernel.bin`: the kernel and the model's device tree, as written to a kernel partition. |
| `kernel-source` | The Linux tree with the port applied and nothing built, for editing. |
| `apps` | The built-in apps, compiled for the phone. |
| `web` | The web frontend bundle that `dphone` embeds. |
| `brush`, `coreutils`, `sed` | The shell and utilities of the root file system. |
| `rootfs-tree`, `rootfs` | The root file system as a directory, and as the JFFS2 image in the EFF image. |
| `dphone-image` | The host tool that packs and unpacks EFF images, JFFS2 images, and kernel images. |

Any change under `kernel/` rebuilds the whole kernel: a Nix build cannot reuse object files from an earlier build.

To show the model and version of an EFF image:

```sh
nix run .#dphone-image -- eff info result
```

## Cargo workspace

`nix develop` opens a shell with the Nix build's Rust toolchain, the targets for the phone and for WebAssembly, and the other tools that the build scripts and the web frontend need.

Three Open DPMA crates (`dpma-sys`, `dpma-ast`, and `res_open_dpma`) read the headers of an Asterisk source tree and build only with `ASTERISK_SOURCE_DIR` set, so exclude them from workspace-wide commands:

```sh
cargo build --workspace --exclude dpma-sys --exclude dpma-ast --exclude res_open_dpma
```

`cargo build -p dphone` builds the firmware binary for the workstation, which the [simulator](simulator.md) runs.  For the phone, `.cargo/config.toml` sets the linker and the processor flags of the `arm-unknown-linux-musleabi` target:

```sh
cargo build --release -p dphone --target arm-unknown-linux-musleabi
```

### Web frontend

`dphone` embeds the web frontend from `web/dist`, or a placeholder page if `web/dist` does not exist.  To build the frontend:

```sh
cd web
pnpm install --frozen-lockfile
pnpm build
```

## Tests and checks

To run the tests of the firmware's crates on the workstation:

```sh
cargo test --workspace --exclude dpma-sys --exclude dpma-ast --exclude res_open_dpma
```

Tests that need stock firmware images, an Asterisk server, or the OpenSSH client are skipped when the workstation lacks them, so a passing `cargo test` does not cover everything.

The built-in apps are a separate workspace.  To run the apps' unit tests on the workstation, with the alias from `apps/.cargo/config.toml`:

```sh
cd apps
cargo host-test
```

The flake's checks skip nothing, and CI runs them on every push:

| Command | Description |
| --- | --- |
| `nix build .#checks.x86_64-linux.tests` | Runs the tests of every crate and of the apps, with the stock images, an Asterisk server with Open DPMA, and an SSH client, and requires the tests that need them to run. |
| `nix build .#checks.x86_64-linux.clippy` | Lints the workspace and the apps with Clippy.  Any warning fails the check. |
| `nix build .#checks.x86_64-linux.jffs2-mount` | In a virtual machine, mounts the root file system image with the kernel's JFFS2 driver, on simulated NAND flash and on a RAM device, and compares the files with the source directory.  Requires `/dev/kvm`. |
| `nix flake check` | Runs every check, and builds the kernel, the utilities, and the `dphone` binary. |

CI (`.github/workflows/ci.yml`) also uploads the image of every model as a workflow artifact named `firmware-<model>`, so a pushed branch's images need no local build.

The Open DPMA tests run in containers against each supported Asterisk version: `cargo xtask ci` in `open-dpma/`; see `open-dpma/README.md`.

## xtask commands

| Command | Description |
| --- | --- |
| `cargo xtask deploy <phone>` | Builds `dphone` for the phone, pushes the binary, and restarts the main role.  See [Binary deploy](test-builds.md#binary-deploy). |
| `cargo xtask kernel-sync <linux tree>` | Regenerates `kernel/patches`, `kernel/files`, and `kernel/configs` from a Linux checkout that contains the port.  For the procedure, see `kernel/README.md`. |
| `cargo xtask tz-table` | Regenerates the phone's table of time zone rules from the workstation's zone database. |
