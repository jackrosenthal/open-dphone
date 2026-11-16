# App SDK

Apps for Open DPhone are written in Rust with the `dphone-app-sdk` crate.  An app can build screens from widgets, draw on the display, place and answer calls, talk to the PBX, send HTTP requests, and store data on the phone or on the PBX.

<figure markdown="span">
  ![Fresh Pot on a color display and on a grayscale display](../assets/app-sdk/gauge.png){ width="656" }
  <figcaption>Fresh Pot, the app built in the tutorial, on a D60 and on a D40.</figcaption>
</figure>

## How apps work

An app is a single file: a WebAssembly component built from a Rust crate.  You install the file from the phone's web interface, and the app then appears in the Applications menu next to the built-in apps.

Apps are event-driven.  The phone calls an app once at startup, and then once for every event: a key press, a softkey, a timer, a reply from the PBX.  In response, the app calls the phone to build a screen, set softkeys, start timers, store data, or send requests.  None of these calls block.  A request that takes time returns immediately, and the result arrives later as another event.

Three things hold for every app:

- **One file runs on every model.**  The D60, D62, and D65 have a 480 by 272 color display; the D70 has a 320 by 160 grayscale display; and the D40, D45, and D50 have a 240 by 120 grayscale display.  An app asks the phone for the display's dimensions and colors and lays out each screen to fit.
- **Apps are sandboxed.**  Each app has a memory limit and a time limit per event.  The phone stops an app that exceeds either limit or panics, and calls and other apps keep running.
- **Apps declare no permissions.**  An app gets the interfaces that the code calls, and nothing else.

## Interfaces

Each interface of the phone is a module of `dphone-app-sdk`.

| Area | Interfaces |
| --- | --- |
| Display and keys | [`ui`](reference/ui.md) (widgets, softkeys, and keys), [`canvas`](reference/canvas.md) (free-form drawing), [`input`](reference/input.md) (raw access to every key and the hookswitch), [`lamps`](reference/lamps.md) (the lamps on the keys) |
| Time and data | [`timers`](reference/timers.md) (timers and clocks), [`storage`](reference/storage.md) (data on the phone), [`pbx-storage`](reference/pbx-storage.md) (data on the PBX, shared across phones), [`config`](reference/config.md) (provisioned settings), [`l10n`](reference/l10n.md) (date and time formatting) |
| Telephony | [`calls`](reference/calls.md) (call control and voicemail counts), [`presence`](reference/presence.md) (contact and user presence), [`pbx`](reference/pbx.md) (requests to the PBX's application server), [`events`](reference/events.md) (events pushed by the PBX, and messages between apps), [`hotdesk`](reference/hotdesk.md) (user login and logout) |
| Network and sound | [`http`](reference/http.md) (HTTP and HTTPS requests), [`audio`](reference/audio.md) (sound playback) |
| System | [`runtime`](reference/runtime.md) (logging, exiting, and opening other apps), [`alarms`](reference/alarms.md) (the alarm clock), [`diagnostics`](reference/diagnostics.md) (subsystem health and the log) |

## Prerequisites

- **Rust 1.88 or later**, installed with `rustup`, plus the `wasm32-unknown-unknown` target and the `wasm-tools` command.  The tutorial walks through [installing both](tutorial/first-app.md#toolchain).
- **The `dphone-app-sdk` crate.**  The crate is versioned with the firmware: an app for firmware 3 depends on `dphone-app-sdk = "3"` from crates.io.  For a phone running firmware built from source, depend on the crate in a clone of the repository instead.  The tutorial shows [both forms](tutorial/first-app.md#project-setup).
- **A phone running Open DPhone**, and the password for the phone's web interface.  The web interface installs apps, shows the phone's log, and mirrors the phone's display.
- **A PBX running [Open DPMA](../open-dpma/index.md)**, only if the app stores data on the PBX.

You don't need to know WebAssembly.  The SDK exposes the phone's interface as Rust functions and a single trait.

## Examples

The firmware's twelve [built-in apps](https://github.com/jackrosenthal/open-dphone/tree/main/apps) are written with the same SDK, and the source makes a good set of complete examples.  Hot Desking and Parking are the shortest.
