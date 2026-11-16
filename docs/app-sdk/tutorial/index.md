# Tutorial

This tutorial builds Fresh Pot, an app for the office coffee pot.  Whoever makes coffee presses a softkey on the nearest phone, and every phone in the office then shows when the pot was brewed, who brewed the pot, and how fresh the coffee still is.

<figure markdown="span">
  ![Fresh Pot showing a pot that Alice brewed twelve minutes ago](../../assets/app-sdk/shared.png){ width="480" }
  <figcaption>Fresh Pot on Bob's phone, twelve minutes after Alice pressed Brewed on another phone.</figcaption>
</figure>

You start with an empty directory.  Each of the five chapters adds one feature and ends with an app that builds, installs, and runs.  Every chapter shows each change to the source and closes with the complete `src/lib.rs` at that point.

## Prerequisites

You should know some Rust: structs, enums, traits, and `match`.  You don't need to know WebAssembly or to have written an app for these phones before.

You need:

- A computer with Rust 1.88 or later, installed with [`rustup`](https://rustup.rs/).
- A phone running Open DPhone on the same network as the computer, and the password for the phone's web interface.
- For the last chapter only, a PBX running [Open DPMA](../../open-dpma/index.md).  Without one, the finished app still works, on one phone at a time.
