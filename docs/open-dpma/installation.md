# Installation

This page takes an Asterisk administrator from a server without Open DPMA to a server on which phones are provisioned by the module.

## Requirements

| Requirement | Details |
| --- | --- |
| Asterisk | Version 22, 23, or 24.  Each pre-built module is for one major version. |
| Platform | Linux on x86_64 or aarch64, with glibc 2.34 or newer (e.g., Debian 12, Ubuntu 22.04, or RHEL 9). |
| Asterisk modules | `res_pjsip`, `res_pjsip_session`, and `res_pjsip_pubsub`.  Asterisk refuses to load Open DPMA without the three. |
| SIP channel driver | PJSIP.  Open DPMA does not work with `chan_sip`. |
| `pjsip.conf` | The endpoint that `default_outbound_endpoint` of the `[global]` section names must exist.  The module creates the endpoint that receives the phones' requests from that endpoint's name. |
| `res_digium_phone.conf` | The file must exist in Asterisk's configuration directory.  See [Configuration](configuration.md). |

The pre-built modules are for the versions and platforms above.  For another platform, [build the module from source](#building-from-source).

The module uses `func_presencestate`, `app_voicemail`, `res_parking`, and `app_queue` when Asterisk has loaded them, for the phones' Status, Voicemail, Parking, and Queues apps.

!!! warning "A missing default outbound endpoint stops Asterisk"

    When `pjsip.conf` has no endpoint of the name that `default_outbound_endpoint` gives, Open DPMA fails to load, and Asterisk treats that failure as fatal and does not start.  The original module behaves the same way.  Define the endpoint before installing the module:

    ```ini title="pjsip.conf"
    [global]
    type=global
    default_outbound_endpoint=dpma_endpoint

    [dpma_endpoint]
    type=endpoint
    context=default
    disallow=all
    allow=ulaw
    ```

## Get a release build

Each release on the project's [releases page](https://github.com/jackrosenthal/open-dphone/releases) has one module for each supported Asterisk version and architecture, and a `SHA256SUMS` file.  Open DPMA has the version number of the Open DPhone firmware, and the same release has the firmware images.

| | x86_64 | aarch64 |
| --- | --- | --- |
| Asterisk 22 | `res_open_dpma-asterisk22-x86_64.so` | `res_open_dpma-asterisk22-aarch64.so` |
| Asterisk 23 | `res_open_dpma-asterisk23-x86_64.so` | `res_open_dpma-asterisk23-aarch64.so` |
| Asterisk 24 | `res_open_dpma-asterisk24-x86_64.so` | `res_open_dpma-asterisk24-aarch64.so` |

Download the module for the server and the checksum file, then verify the download.  For Asterisk 22 on x86_64:

```sh
curl -fLO https://github.com/jackrosenthal/open-dphone/releases/latest/download/res_open_dpma-asterisk22-x86_64.so
curl -fLO https://github.com/jackrosenthal/open-dphone/releases/latest/download/SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS
```

`sha256sum` must print `OK` for the module.  `asterisk -V` prints the version of Asterisk, and `uname -m` prints the architecture.

## Install the module

1. Copy the module into Asterisk's module directory under the name `res_open_dpma.so`.  The module directory is `astmoddir` in `asterisk.conf`, e.g., `/usr/lib/asterisk/modules`.

    ```sh
    install -m 0755 res_open_dpma-asterisk22-x86_64.so /usr/lib/asterisk/modules/res_open_dpma.so
    ```

2. On a server that has the original module, [stop Asterisk from loading the original](#replace-the-original-module).
3. Make sure that `res_digium_phone.conf` exists.  A server that had the original module keeps the existing file.  For a new server, start from the [minimal configuration](configuration.md#a-minimal-configuration).
4. Restart Asterisk.

## Replace the original module

Open DPMA and the original module cannot both be loaded: both register the same CLI commands, AMI actions, and message technology, and both read `res_digium_phone.conf`.  Open DPMA contains the function of all three modules of the original, so all three have to stay unloaded.

Add the following lines to `modules.conf`, or remove the three files from the module directory:

```ini title="modules.conf"
noload => res_digium_phone.so
noload => res_pjsip_endpoint_identifier_dpma.so
noload => res_pjsip_dialog_info_digium_body_supplement.so
```

The configuration file needs no change, with two exceptions: a server whose lines come from `chan_sip` has to move those lines to PJSIP first, and scripts that reload the module have to use the name `res_open_dpma.so`.  [Differences from the original module](index.md#differences-from-the-original-module) lists everything else that changes.

To return to the original module, remove `res_open_dpma.so` from the module directory, remove the three `noload` lines, and restart Asterisk.

## Confirm that the module works

Run the commands below in the Asterisk console (`asterisk -r`).

1. The module is loaded, and reports the version:

    ```text
    *CLI> module show like res_open_dpma
    *CLI> digium_phones show version
    Open DPMA version 3.0.0
    ```

2. The settings of `[general]` are the intended ones, in particular the two authentication lines:

    ```text
    *CLI> digium_phones show settings
    ```

3. Every phone section of the configuration file is listed:

    ```text
    *CLI> digium_phones show phones
    ```

4. After a phone has started, the phone has a session with `Auth:Yes` and `Configured: Yes`, and a configuration token:

    ```text
    *CLI> digium_phones show sessions
    *CLI> digium_phones show tokens
    ```

5. The phone's lines are registered:

    ```text
    *CLI> pjsip show contacts
    ```

A phone finds the server from the module's mDNS announcement when service discovery is enabled and the phone and the server are on the same network segment; see [`service_discovery_enabled`](configuration.md#general).

!!! tip "See the module's log messages"

    The module writes most messages at a log level of the module's, named `DPMA`.  Asterisk shows the level only in a log channel that lists the level:

    ```ini title="logger.conf"
    [logfiles]
    console => notice,warning,error,dpma
    messages.log => notice,warning,error,dpma
    ```

    The reason that a phone was refused a configuration is logged at that level, e.g., `Auth failed for ip ...`.

## Building from source

Build the module from source for a platform that the releases do not cover, or to run a change.  The build needs Rust 1.99 or newer, libclang, and the configured headers of the Asterisk version that the module is for: the source tree of that Asterisk after `./configure` has run, named by the environment variable `ASTERISK_SOURCE_DIR`.

```sh
git clone https://github.com/jackrosenthal/open-dphone.git
cd open-dphone/open-dpma
export ASTERISK_SOURCE_DIR=/usr/src/asterisk-22.11.0
cargo build --profile module -p res_open_dpma
```

The module is written to `target/module/libres_open_dpma.so` in the repository's root directory.  Install that file as `res_open_dpma.so`, as described in [Install the module](#install-the-module).

With podman or docker installed, `cargo xtask headers` supplies the headers instead: the command copies the configured headers out of the container image of the Asterisk release that the project tests against, and prints the directory.

```sh
export ASTERISK_SOURCE_DIR=$(cargo xtask headers --asterisk 22)
```

!!! warning "Do not enable the `devtest` feature"

    The crate's `devtest` feature adds the `dpmadev` CLI commands that the test suites use, e.g., commands that issue configuration tokens and send messages to phones.  A module for a production server must be built without the feature, which is the default.

The [Dev Guide](../dev-guide/index.md) covers the rest of the repository.
