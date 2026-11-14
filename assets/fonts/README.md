# Fonts

- `Roboto-*.ttf`: Roboto 3.016 from the upstream release (as packaged by Arch Linux `ttf-roboto`), SIL Open Font License 1.1 (`LICENSE-Roboto-OFL.txt`).  Used by the color skin.
- `DroidSans*.ttf`: Droid Sans (Android Open Source Project, 2012-10-17), Apache License 2.0 (`LICENSE-DroidSans-Apache-2.0.txt`).  Used by the gray skins.

`crates/dphone-ui/build.rs` rasterizes these into glyph atlases at build time; the TrueType files are not shipped on the phone.
