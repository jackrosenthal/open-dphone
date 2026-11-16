# Installation

## Requirements

| Requirement | Details |
| --- | --- |
| Asterisk | Version 22, 23, or 24.  Each prebuilt module targets one major version. |
| Platform | Linux on x86_64 or aarch64, with glibc 2.34 or later (e.g., Debian 12, Ubuntu 22.04, or RHEL 9).  For other platforms, [build from source](#building-from-source). |
| Asterisk modules | `res_pjsip`, `res_pjsip_session`, and `res_pjsip_pubsub`.  Asterisk refuses to load Open DPMA without all three. |
| SIP channel driver | PJSIP.  Open DPMA does not support `chan_sip`. |
| `pjsip.conf` | The endpoint named by `default_outbound_endpoint` in the `[global]` section must exist.  The module creates the internal endpoint that receives phone requests under the same name. |
| `res_digium_phone.conf` | The file must exist in the Asterisk configuration directory.  See the [configuration reference](configuration.md). |

The phones' Status, Voicemail, Parking, and Queues apps rely on `func_presencestate`, `app_voicemail`, `res_parking`, and `app_queue`.  These modules are optional, and Open DPMA uses whichever ones Asterisk has loaded.

!!! warning "A missing default outbound endpoint prevents Asterisk from starting"

    If `pjsip.conf` does not define the endpoint that `default_outbound_endpoint` names, Open DPMA fails to load.  Asterisk treats that failure as fatal and exits.  The original module behaves the same way.  Define the endpoint before installing the module:

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

## Download

Each release on the [releases page](https://github.com/jackrosenthal/open-dphone/releases) includes one module for each supported Asterisk version and architecture, along with a `SHA256SUMS` file.  Open DPMA shares a version number with the Open DPhone firmware, and the firmware images are part of the same release.

| | x86_64 | aarch64 |
| --- | --- | --- |
| Asterisk 22 | `res_open_dpma-asterisk22-x86_64.so` | `res_open_dpma-asterisk22-aarch64.so` |
| Asterisk 23 | `res_open_dpma-asterisk23-x86_64.so` | `res_open_dpma-asterisk23-aarch64.so` |
| Asterisk 24 | `res_open_dpma-asterisk24-x86_64.so` | `res_open_dpma-asterisk24-aarch64.so` |

Run `asterisk -V` to find the Asterisk version and `uname -m` to find the architecture.  Then download the matching module and the checksum file, and verify the download.  For Asterisk 22 on x86_64:

```sh
curl -fLO https://github.com/jackrosenthal/open-dphone/releases/latest/download/res_open_dpma-asterisk22-x86_64.so
curl -fLO https://github.com/jackrosenthal/open-dphone/releases/latest/download/SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS
```

`sha256sum` must print `OK` for the module.

## Setup

1. Copy the module into the Asterisk module directory as `res_open_dpma.so`.  The module directory is `astmoddir` in `asterisk.conf`, e.g., `/usr/lib/asterisk/modules`.

    ```sh
    install -m 0755 res_open_dpma-asterisk22-x86_64.so /usr/lib/asterisk/modules/res_open_dpma.so
    ```

2. If the server has the original module, [disable the original module](#migration-from-the-original-module).
3. Make sure that `res_digium_phone.conf` exists.  A server that ran the original module already has the file.  On a new server, start with the [minimal example](configuration.md#minimal-example).
4. Restart Asterisk.

## Migration from the original module

Open DPMA and the original module cannot be loaded at the same time.  Both register the same CLI commands, AMI actions, and message technology, and both read `res_digium_phone.conf`.  Open DPMA replaces all three of the original modules, so none of the three may be loaded.

Add the following lines to `modules.conf`, or delete the three files from the module directory:

```ini title="modules.conf"
noload => res_digium_phone.so
noload => res_pjsip_endpoint_identifier_dpma.so
noload => res_pjsip_dialog_info_digium_body_supplement.so
```

The configuration file needs no changes, with two exceptions:

- If the server's lines use `chan_sip`, move the lines to PJSIP first.
- Scripts that reload the module must use the name `res_open_dpma.so`.

See [Differences from the original module](index.md#differences-from-the-original-module) for everything else that changes.

To switch back to the original module, delete `res_open_dpma.so` from the module directory, remove the three `noload` lines, and restart Asterisk.

## Verification

Run the following commands in the Asterisk console (`asterisk -r`).

1. Check that the module is loaded and reports a version:

    ```text
    *CLI> module show like res_open_dpma
    *CLI> digium_phones show version
    Open DPMA version 3.0.0
    ```

2. Check the `[general]` settings, especially the two authentication lines:

    ```text
    *CLI> digium_phones show settings
    ```

3. Check that every phone section in the configuration file is listed:

    ```text
    *CLI> digium_phones show phones
    ```

4. Start a phone, then check that the phone has a session with `Auth:Yes` and `Configured: Yes`, and a configuration token:

    ```text
    *CLI> digium_phones show sessions
    *CLI> digium_phones show tokens
    ```

5. Check that the phone's lines are registered:

    ```text
    *CLI> pjsip show contacts
    ```

With service discovery enabled, phones on the same network segment as the server find the server through the module's mDNS announcement.  See [`service_discovery_enabled`](configuration.md#general).

## Logging

The module logs most messages at the `DPMA` log level, which the module adds to Asterisk.  Asterisk writes those messages only to log channels that list `dpma`:

```ini title="logger.conf"
[logfiles]
console => notice,warning,error,dpma
messages.log => notice,warning,error,dpma
```

Messages at that level explain why a phone was refused a configuration, e.g., `Auth failed for ip ...`.

## Building from source

Build from source to run the module on a platform that the releases do not cover, or to test a change.  You need Rust 1.99 or later, libclang, and the configured headers for the target Asterisk version, i.e., the Asterisk source tree after `./configure` has run.  Set the environment variable `ASTERISK_SOURCE_DIR` to that source tree.

```sh
git clone https://github.com/jackrosenthal/open-dphone.git
cd open-dphone/open-dpma
export ASTERISK_SOURCE_DIR=/usr/src/asterisk-22.11.0
cargo build --profile module -p res_open_dpma
```

The build writes the module to `target/module/libres_open_dpma.so` in the repository's root directory.  Install that file as `res_open_dpma.so`, as described in [Setup](#setup).

If you have podman or docker installed, `cargo xtask headers` can supply the headers instead.  The command copies the configured headers out of the container image for the Asterisk release that the project tests against, and prints the directory:

```sh
export ASTERISK_SOURCE_DIR=$(cargo xtask headers --asterisk 22)
```

!!! warning "Do not enable the `devtest` feature"

    The crate's `devtest` feature adds the `dpmadev` CLI commands used by the test suites, which include commands that issue configuration tokens and send messages to phones.  Never enable the feature in a production build.  The feature is off by default.
