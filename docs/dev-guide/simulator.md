# Simulator

Run the commands on this page from the root of the repository, in a [`nix develop` shell](building.md#cargo-workspace).

## Phone simulator

`dphone main --sim` runs the main role on the hardware layer's simulator backend: the user interface draws to an in-memory frame buffer, the audio devices are simulated, and the workstation's network configuration is untouched.  The configuration, the SIP stack, the DPMA client, the app runtime, the web interface, and the SSH server are the phone's code.

```sh
cargo run -p dphone -- main --sim --model d60 --nvdata /tmp/dphone-sim
```

`--nvdata` names the directory that serves as nvdata.  `--model` accepts any of the seven models; the default is `d60`.  The main role logs to the terminal.

| Interface | Details |
| --- | --- |
| Web interface | `http://127.0.0.1:8080/`, or the port given by `--web-port`.  A new nvdata directory has the default login: user `admin`, password `789`.  The **Screen** page mirrors the simulated display and takes key presses. |
| SIP | Binds port 5060 on the workstation.  As with an empty nvdata on a phone, the simulator starts at the menu that asks for a server.  Provision the simulator from a DPMA server, or add SIP accounts in the web interface. |
| SSH | Only with `--ssh-listen <address>`, e.g., `--ssh-listen 127.0.0.1:2222`, and only while the [SSH setting](ssh-and-serial-console.md#ssh) is on. |
| Apps | The built-in apps are missing unless the environment variable `DPHONE_APPS_ROOT` names a directory of staged apps.  For staging, see `crates/dphone-apps/README.md`. |

### `dphone` commands { #dphone-commands }

Commands that the main role answers reach the simulator through a socket in the run directory: `.run` in the nvdata directory, or the directory given by `--run-dir`.  Set the environment variable `DPHONE_DIAG_SOCKET` to the socket's path:

```sh
export DPHONE_DIAG_SOCKET=/tmp/dphone-sim/.run/diag.sock
cargo run -q -p dphone -- ping
cargo run -q -p dphone -- key 1 2 3
cargo run -q -p dphone -- screenshot /tmp/screen.ppm
cargo run -q -p dphone -- log -f
```

Commands that touch flash or the kernel, such as `dphone update` and `dphone ramboot`, require a phone.  For all commands, see the [`dphone` command reference](dphone-command.md).

!!! note "Keep the nvdata path short"

    Unix socket paths are limited to about 100 characters.  With a longer path, the main role logs that the socket path is too long, and commands cannot reach the simulator.  Use a shorter nvdata directory or a short `--run-dir`.

## Web interface simulator

An example program in `crates/dphone-web` runs the web server on a state store filled with mock calls, accounts, contacts, and apps.  Commands behave plausibly: a dialed call rings and connects, and a firmware installation steps through the phases.

```sh
cargo run -p dphone-web --example sim -- --model d60 --port 8480
```

The interface is at `http://127.0.0.1:8480/`; the login is `admin`, password `789`.

For live reload, run the Vite development server, which proxies API requests to the example program on port 8480:

```sh
cd web
pnpm install --frozen-lockfile
pnpm dev
```

To use a real phone instead, set the environment variable `DPHONE` to the phone's URL, e.g., `DPHONE=http://192.0.2.20 pnpm dev`.

After changing the service definitions in `proto/`, run `pnpm gen` in `web/` to regenerate the committed `web/src/gen`.
