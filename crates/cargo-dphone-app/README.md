# cargo-dphone-app

The tool that builds apps for [Open DPhone](https://github.com/jackrosenthal/open-dphone), the open firmware for Digium D-series desk phones.  Apps are written with the crate [`dphone-app-sdk`](https://crates.io/crates/dphone-app-sdk), whose README tells how; this tool turns such a crate into the one file the phone installs.

The version of this crate is the version of the firmware it builds apps for, as with the SDK.

```text
cargo install cargo-dphone-app
rustup target add wasm32-unknown-unknown
```

## Use

In the directory of an app (a `cdylib` crate that registers itself with `export_app!` of the SDK):

```text
cargo dphone-app
```

This runs `cargo build --release --target wasm32-unknown-unknown`, makes a WebAssembly component of the module cargo built, and writes the app, `target/dphone-app/<name>.wasm`, where `<name>` is the app's name in its manifest.  The manifest and the app's data files are in the file: `export_app!` put them into the module, and they stay in the component.

- `-o <file>` writes the app there instead.
- `-p <crate>` builds one app in a workspace of several (without `-p`, every app of the workspace is built), and `--manifest-path <path>` names the `Cargo.toml`.
- Other options for `cargo build` go after `--`, e.g., `cargo dphone-app -- --offline`.

`cargo dphone-app build` is the same, spelled out, and `cargo dphone-app --help` lists everything.

The tool reads the file as the phone will and refuses what the phone would refuse: a manifest the phone cannot read, or an app that uses an interface reserved for the firmware's built-in apps.  It prints the app's name and the interfaces of the phone the app uses (its capabilities, which the phone takes from the component's imports).

Install the file in the phone's web interface: the page "Apps", the button "Install App".

## The second step on its own

```text
cargo dphone-app componentize <module.wasm> -o <app.wasm>
```

makes the app of a module that was built some other way.

## As a library

Without the default feature `tool`, the crate has no dependencies and is the reader of the manifest alone (`manifest`): the firmware reads app files with it, so the tool and the phone agree on what an app is.

## License

GPL-2.0-or-later, as the firmware.
