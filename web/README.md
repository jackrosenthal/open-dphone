# Open DPhone web frontend

TypeScript, React, Tailwind CSS, and Vite; RPC through `@connectrpc/connect-web`
and `@connectrpc/connect-query`, with code generated from `../proto` by buf.

```text
pnpm install --frozen-lockfile
pnpm build        # type-check, then write dist/
pnpm dev          # Vite dev server; proxies the API to the host simulator
pnpm gen          # regenerate src/gen from ../proto (needs buf)
```

`crates/dphone-web/build.rs` embeds `dist/` precompressed (gzip and brotli) when
it exists, and a placeholder page otherwise.  Set `DPHONE_WEB_DIST` to embed a
bundle from another directory (the Nix build does).

The host simulator serves the same API with made-up state:

```text
cargo run -p dphone-web --example sim -- --model d60 --port 8480
```

`src/gen` is committed, so a build needs neither buf nor the plugin.
