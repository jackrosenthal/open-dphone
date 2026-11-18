//! From the module cargo builds for a built-in app to the app file: a
//! component.  This is what `wasm-tools component new` does for an app
//! that is installed.

use crate::manifest::{Capability, Manifest};

/// Wraps a core WebAssembly module built with `opendeskphone-app-sdk` into a
/// component.  The module carries the interface it was built against, and
/// its manifest stays in it, so nothing else is needed.
pub fn componentize(module: &[u8]) -> Result<Vec<u8>, String> {
    wit_component::ComponentEncoder::default()
        .module(module)
        .map_err(|e| format!("{e:#}"))?
        .validate(true)
        .encode()
        .map_err(|e| format!("{e:#}"))
}

/// The interfaces of the phone a component imports.
fn capabilities(component: &[u8]) -> Result<Vec<Capability>, String> {
    let decoded = wit_component::decode(component).map_err(|e| format!("{e:#}"))?;
    let wit_component::DecodedWasm::Component(resolve, world) = &decoded else {
        return Err("not a component".into());
    };
    let mut found = Vec::new();
    for key in resolve.worlds[*world].imports.keys() {
        let import = resolve.name_world_key(key);
        if let Some(c) = Capability::of_import(&import).map_err(|e| e.to_string())?
            && !found.contains(&c)
        {
            found.push(c);
        }
    }
    Ok(found)
}

/// An app as the phone will see it.
pub struct BuiltApp {
    /// The manifest, with the capabilities: the interfaces the component
    /// imports.
    pub manifest: Manifest,
    /// The app file: the component.
    pub component: Vec<u8>,
}

/// Makes the app file of a module cargo built, and reads it as the phone
/// will.
pub fn build(module: &[u8]) -> Result<BuiltApp, String> {
    let component = componentize(module)?;
    let mut manifest = Manifest::from_wasm(&component).map_err(|e| e.to_string())?;
    manifest.capabilities = capabilities(&component)?;
    Ok(BuiltApp {
        manifest,
        component,
    })
}
