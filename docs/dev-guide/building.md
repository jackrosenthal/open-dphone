# Building from source

This page shows where the parts of the firmware are in the repository, how to build a firmware image for a phone, and how to run the tests and checks that a change must pass.

## What the build needs

- A Linux machine with an x86-64 processor.  The Nix flake defines the builds for `x86_64-linux` only.
- [Nix](https://nixos.org/download/), with the `nix-command` and `flakes` features enabled.  Nix supplies every other tool, including the Rust toolchain, the ARM C compiler for the kernel, and Node.js for the web interface.
- A network connection for the first build.  Nix downloads the Linux 7.3 source and two stock firmware images from the vendor's server.  The image build takes the voice DSP firmware out of the stock images, because the DSP firmware is proprietary and is not in the repository.  Later builds reuse the downloads from the Nix store.

```sh
git clone https://github.com/jackrosenthal/open-dphone.git
cd open-dphone
```

## The repository

| Path | Contents |
| --- | --- |
| `crates/dphone` | The firmware's one program: the init role, the main role, the SSH server, and the [`dphone` command](dphone-command.md). |
| `crates/dphone-early` | The early-boot stage that the kernel image carries. |
| `crates/dphone-update`, `crates/dphone-ctrl` | The firmware updater, the trial boot, and `dphone ramboot`. |
| `crates/dphone-hal` | The hardware layer, with one backend for the phone and one for the simulator. |
| `crates/dphone-ui`, `crates/dphone-sip`, `crates/dphone-media`, `crates/dphone-vpm`, `crates/dphone-net`, `crates/dphone-bt`, `crates/dphone-dpma`, `crates/dphone-config`, `crates/dphone-state` | The subsystems of the main role: the screens, SIP, audio and the voice DSP, the network, Bluetooth, the DPMA client, the configuration, and the state store that connects the subsystems. |
| `crates/dphone-apps`, `crates/dphone-app-sdk`, `apps/` | The app runtime, the SDK for app writers, and the built-in apps.  `apps/` is a separate Cargo workspace that builds for WebAssembly. |
| `crates/dphone-web`, `web/`, `proto/` | The web server, the web frontend (TypeScript and React), and the service definitions that both are generated from. |
| `crates/dphone-image`, `crates/eff`, `crates/jffs2-image` | The host tool that builds and inspects firmware images, and the libraries for the image formats. |
| `kernel/` | The port of Linux 7.3 to the phones, as patches and added files.  `kernel/README.md` describes the constraints of the kernel and the way to work on the port. |
| `nix/`, `flake.nix` | The build recipes. |
| `open-dpma/` | Open DPMA, the module for Asterisk, with a separate README. |
| `xtask/` | `cargo xtask`, the developer commands that are not builds. |
| `assets/` | The fonts, icons, and artwork that the user interface is built with. |
| `docs/`, `zensical.toml` | This documentation site. |

## Build a firmware image

Each model has one flake output, `eff-<model>`, for the models `d40`, `d45`, `d50`, `d60`, `d62`, `d65`, and `d70`.  From the root of the repository:

```sh
nix build .#eff-d60
```

The result is the image file.  `result` in the current directory is a link to the file in the Nix store, and the file's name there has the form `<version>_<model>_firmware.eff`, e.g., `3_0_0_D60_firmware.eff`.  To print the path and create no link:

```sh
nix build .#eff-d60 --no-link --print-out-paths
```

`nix build .#all` builds the images of all seven models into one directory.  The version in the name is the workspace version in the root `Cargo.toml`.

!!! note "Nix builds only what git tracks"
    A flake in a git repository sees the files that git tracks, with the changes in the working tree.  A new file is invisible to the build until `git add` has added the file.

The kernel, the root file system, and the `dphone` binary are the same for every model.  The images differ in the device tree that is joined to the kernel and in the model name in the image header.  A phone refuses an image made for another model.

### The parts of an image

Each part of an image is a flake output and can be built alone, which is quicker than a whole image when only one part is of interest.

| Output | Result |
| --- | --- |
| `dphone` | The firmware binary for the phone: static, for ARMv6. |
| `dphone-early` | The early-boot stage. |
| `kernel` | The kernel (`zImage`), a device tree for each model, and the loadable modules. |
| `kernel-<model>` | `kernel.bin`, the kernel and the model's device tree as written to a kernel partition. |
| `kernel-source` | The Linux 7.3 tree with the port applied and nothing built, for editing. |
| `apps` | The built-in apps, compiled for the phone. |
| `web` | The web frontend bundle that `dphone` embeds. |
| `brush`, `coreutils`, `sed` | The shell and the utilities of the root file system. |
| `rootfs-tree`, `rootfs` | The root file system as a directory, and as the JFFS2 image that goes into the EFF image. |
| `dphone-image` | The host tool that packs and unpacks EFF images, JFFS2 images, and kernel images. |

A change anywhere under `kernel/` builds the whole kernel again, because a Nix build cannot reuse the object files of an earlier build.

`dphone-image` answers questions about an image file without a phone, e.g., which model and version an EFF image is for:

```sh
nix run .#dphone-image -- eff info result
```

## The Cargo workspace

Nix builds the images, and Cargo is the tool for the edit, build, and test cycle on the workstation.  `nix develop` opens a shell with the Rust toolchain of the build, including the targets for the phone and for WebAssembly, and with the other tools that the workspace's build scripts and the web frontend need:

```sh
nix develop
```

The root `Cargo.toml` lists the firmware's crates, `xtask`, and the crates of Open DPMA.  Three of the Open DPMA crates (`dpma-sys`, `dpma-ast`, and `res_open_dpma`) read the headers of an Asterisk source tree and build only with `ASTERISK_SOURCE_DIR` set, so a command over the whole workspace leaves the three out:

```sh
cargo build --workspace --exclude dpma-sys --exclude dpma-ast --exclude res_open_dpma
```

A build of one crate needs no exclusions.  `cargo build -p dphone` builds the firmware binary for the workstation, which is the binary that the [simulator](simulator.md) runs.

The binary for the phone is built for the target `arm-unknown-linux-musleabi`.  `.cargo/config.toml` holds the linker and the processor flags, so the command needs only the target:

```sh
cargo build --release -p dphone --target arm-unknown-linux-musleabi
```

[`cargo xtask deploy`](ramboot.md#push-a-changed-binary) runs this build and pushes the result to a phone.  The release profile optimizes the whole program as one unit, so this build is much slower than a debug build.

`dphone` embeds the web frontend from `web/dist`.  When that directory does not exist, a Cargo build embeds a placeholder page.  To build the frontend first:

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

Some tests compare against stock firmware images, register against an Asterisk server, or log in with the OpenSSH client.  Those tests skip themselves when the workstation does not provide what the tests need, so a passing `cargo test` does not cover everything.

The built-in apps are a separate workspace, and the apps' unit tests run on the workstation with an alias defined in `apps/.cargo/config.toml`:

```sh
cd apps
cargo host-test
```

The flake's checks run everything with nothing skipped, and are what the continuous integration runs for every push:

| Command | What the check does |
| --- | --- |
| `nix build .#checks.x86_64-linux.tests` | Runs the tests of every crate and of the apps, with the stock images, an Asterisk server with Open DPMA, and an SSH client present, and requires the tests that depend on those to run. |
| `nix build .#checks.x86_64-linux.clippy` | Lints the workspace and the apps with Clippy.  A warning fails the check. |
| `nix build .#checks.x86_64-linux.jffs2-mount` | Mounts the root file system image with the kernel's JFFS2 driver in a virtual machine, on simulated NAND flash and on a RAM device, and compares the files with the source directory.  The check needs `/dev/kvm`. |
| `nix flake check` | Runs every check, and builds the kernel, the utilities, and the `dphone` binary. |

The continuous integration (`.github/workflows/ci.yml`) also builds the image of every model and uploads each image as a workflow artifact named `firmware-<model>`, so a pushed branch yields images without a local build.

The tests of Open DPMA run in a container against each supported Asterisk version, with `cargo xtask ci` in `open-dpma/`.  `open-dpma/README.md` describes that command.

## Other developer commands

`cargo xtask` holds the developer commands that are not builds:

| Command | Purpose |
| --- | --- |
| `cargo xtask deploy <phone>` | Builds `dphone` for the phone, pushes the binary, and restarts the main role.  See [Trying a build without installing](ramboot.md#push-a-changed-binary). |
| `cargo xtask kernel-sync <linux tree>` | Regenerates `kernel/patches`, `kernel/files`, and `kernel/configs` from a checkout of Linux 7.3 that holds the port.  `kernel/README.md` describes the procedure. |
| `cargo xtask tz-table` | Regenerates the phone's table of time zone rules from the workstation's zone database. |
