# Simulator

Two simulators run on a workstation, with no phone required:

- The [phone simulator](#phone-simulator) runs the main role on simulated hardware, for work on anything above the hardware layer.
- The [web interface simulator](#web-interface-simulator) runs the web interface with mock data, for work on the web frontend.

Run the commands on this page from the root of the repository, in a [`nix develop` shell](building.md#cargo-workspace).

## Phone simulator

`dphone main --sim` runs the real main role on the hardware layer's simulator backend.  The user interface draws to an in-memory frame buffer, the audio devices are simulated, and the workstation's network configuration is left untouched.  Everything else is the same code that runs on a phone: the configuration, the SIP stack, the DPMA client, the app runtime, the web interface, and the SSH server.

The simulator needs a directory to use as nvdata:

```sh
cargo run -p dphone -- main --sim --model d60 --nvdata /tmp/dphone-sim
```

`--model` accepts any of the seven models and defaults to `d60`.  The main role logs to the terminal.

While the simulator is running:

- **Web interface.**  The web interface is at `http://127.0.0.1:8080/`; use `--web-port` to choose another port.  A new nvdata directory has the phones' default login: user `admin`, password `789`.  The **Screen** page mirrors the simulated display and lets you press keys.  The frontend is available only after `pnpm build` in `web/` has produced `web/dist`.  Otherwise, `dphone` serves a placeholder page.
- **SIP.**  The SIP stack binds port 5060 on the workstation.  Like a phone with an empty nvdata, the simulated phone starts at the menu that asks for a server.  Provision the simulated phone from a DPMA server, or add SIP accounts in the web interface.
- **SSH.**  The SSH server is available only when you pass `--ssh-listen` with an address, e.g., `--ssh-listen 127.0.0.1:2222`.  The server starts listening once the [SSH setting](ssh-and-serial-console.md#ssh) is on.
- **Apps.**  The built-in apps are missing unless the environment variable `DPHONE_APPS_ROOT` names a directory of staged apps.  See `crates/dphone-apps/README.md` for how to stage the apps.

### `dphone` commands { #dphone-commands }

Commands that the main role answers reach the simulator through a socket in the run directory.  The run directory is `.run` inside the nvdata directory, unless `--run-dir` names another.  Set the environment variable `DPHONE_DIAG_SOCKET` to the socket's path:

```sh
export DPHONE_DIAG_SOCKET=/tmp/dphone-sim/.run/diag.sock
cargo run -q -p dphone -- ping
cargo run -q -p dphone -- key 1 2 3
cargo run -q -p dphone -- screenshot /tmp/screen.ppm
cargo run -q -p dphone -- log -f
```

See the [`dphone` command reference](dphone-command.md) for the full list.  Commands that touch flash or the kernel, such as `dphone update` and `dphone ramboot`, require a phone.

!!! note "Keep the nvdata path short"

    Unix socket paths are limited to about 100 characters.  If the nvdata path is too long, the main role logs that the socket path is too long, and commands cannot reach the simulator.  Use a shorter directory, or pass a short `--run-dir`.

## Web interface simulator

An example program in `crates/dphone-web` runs the real web server on top of a state store filled with mock calls, accounts, contacts, and apps.  Commands behave plausibly: a dialed call rings and connects, and a firmware installation steps through the phases.

```sh
cargo run -p dphone-web --example sim -- --model d60 --port 8480
```

The interface is at `http://127.0.0.1:8480/`.  Log in as `admin` with the password `789`.

For live reload, run the Vite development server, which proxies API requests to the example program on port 8480:

```sh
cd web
pnpm install --frozen-lockfile
pnpm dev
```

To develop the frontend against a real phone instead, set the environment variable `DPHONE` to the phone's URL, e.g., `DPHONE=http://192.0.2.20 pnpm dev`.

### Frontend commands

| Command | Description |
| --- | --- |
| `pnpm build` | Type-checks the sources, then writes the bundle to `web/dist`. |
| `pnpm lint` | Runs ESLint on the sources. |
| `pnpm gen` | Regenerates `web/src/gen` from the service definitions in `proto/`.  The generated code is committed, so run this command only after changing `proto/`. |
