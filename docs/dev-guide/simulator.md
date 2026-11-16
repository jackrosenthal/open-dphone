# Running on a workstation

Much of the firmware runs on the workstation without a phone.  This page shows the two simulators: the main role with simulated hardware, for work on anything above the hardware layer, and the web interface over made-up state, for work on the web frontend.

The commands run from the root of the repository, in the shell that `nix develop` opens (see [Building from source](building.md#the-cargo-workspace)).

## The main role with simulated hardware

`dphone main --sim` runs the real main role on the simulator backend of the hardware layer: the user interface draws to a frame buffer in memory, the audio devices are simulated, and the workstation's network is left as found.  The configuration, the SIP stack, the DPMA client, the app runtime, the web interface, and the SSH server are the code that runs on a phone.

The simulator needs a directory to keep nvdata in:

```sh
cargo run -p dphone -- main --sim --model d60 --nvdata /tmp/dphone-sim
```

`--model` takes any of the seven models and defaults to `d60`.  The main role logs to the terminal.  While the simulator runs:

- The web interface is at `http://127.0.0.1:8080/`.  `--web-port` chooses another port.  A new nvdata directory has the default login of the phones: user `admin`, password `789`.  The **Screen** page mirrors the simulated display and presses keys.  The page shows the frontend only after `pnpm build` in `web/` has produced `web/dist`; a `dphone` built without `web/dist` serves a placeholder page.
- The SIP stack binds port 5060 on the workstation.  The simulated phone starts at the menu that asks for a server, as a phone with empty nvdata does, and can be provisioned from a DPMA server or given SIP accounts in the web interface.
- The SSH server exists only when `--ssh-listen` gives an address, e.g., `--ssh-listen 127.0.0.1:2222`, and listens once the SSH setting is on (see [SSH and the serial console](debug-access.md#ssh)).
- The built-in apps are absent unless the environment variable `DPHONE_APPS_ROOT` names a directory of staged apps.  `crates/dphone-apps/README.md` describes how to stage the apps.

### The `dphone` command against the simulator

The commands that the main role answers reach the simulator through a socket in the run directory, which is `.run` inside the nvdata directory unless `--run-dir` names another.  The environment variable `DPHONE_DIAG_SOCKET` tells the command where the socket is:

```sh
export DPHONE_DIAG_SOCKET=/tmp/dphone-sim/.run/diag.sock
cargo run -q -p dphone -- ping
cargo run -q -p dphone -- key 1 2 3
cargo run -q -p dphone -- screenshot /tmp/screen.ppm
cargo run -q -p dphone -- log -f
```

`key` presses keys of the keypad, `screenshot` writes the display to a picture file, and `log -f` follows the log.  [The `dphone` command](dphone-command.md) lists the rest.  The commands that touch flash or the kernel, e.g., `dphone update` and `dphone ramboot`, need a phone.

!!! note "Keep the nvdata path short"
    A Unix socket path is limited to about 100 characters.  With a long nvdata path the main role logs that the socket path is too long, and the commands cannot reach the simulator.  Choose a shorter directory, or give a short `--run-dir`.

## The web interface over made-up state

For work on the web frontend, an example program in `crates/dphone-web` runs the real web server over a state store filled with invented calls, accounts, contacts, and apps.  Commands behave plausibly: a dialed call rings and connects, and a firmware installation counts through the phases.

```sh
cargo run -p dphone-web --example sim -- --model d60 --port 8480
```

The interface is at `http://127.0.0.1:8480/`, with the user `admin` and the password `789`.

The Vite development server reloads the frontend on every change and passes the API requests through to the example program on port 8480:

```sh
cd web
pnpm install --frozen-lockfile
pnpm dev
```

To develop the frontend against a real phone instead, name the phone in the environment variable `DPHONE`, e.g., `DPHONE=http://192.0.2.20 pnpm dev`.

The other frontend commands:

| Command | Purpose |
| --- | --- |
| `pnpm build` | Checks the types, then writes the bundle to `web/dist`. |
| `pnpm lint` | Runs ESLint over the sources. |
| `pnpm gen` | Regenerates `web/src/gen` from the service definitions in `proto/`.  The generated code is committed, so only a change to `proto/` needs this command. |
