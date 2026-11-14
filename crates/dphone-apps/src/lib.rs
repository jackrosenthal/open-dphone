//! The Open DPhone app runtime: WebAssembly components interpreted by
//! Wasmtime's Pulley interpreter, the host side of the WIT interface in
//! `wit/app.wit`, app packages, and the [`AppManager`] the main role runs.
//!
//! # For the integrator
//!
//! ```ignore
//! let services = Services {
//!     pbx: Arc::new(my_dpma_adapter),        // impl PbxRequests (dasp://)
//!     http: Arc::new(my_http_client),        // impl HttpClient
//!     config: Arc::new(my_config_adapter),   // impl ConfigSource
//! };
//! let mut apps = AppManager::new(ManagerConfig::phone(), store.clone(), Box::new(my_shell), services)?;
//! apps.scan();                // discover built-in and installed apps
//! std::thread::spawn(move || apps.run());   // or call apps.pump() from your own loop
//! ```
//!
//! The manager registers as the handler of [`dphone_state::Domain::Apps`]
//! (`LaunchApp`, `CloseApp`, `InstallApp`, `RemoveApp`), keeps
//! [`dphone_state::Apps`] current, delivers state changes and pushes to
//! apps, and drives each app's window through the [`Shell`] and
//! [`AppWindow`] traits; with the `ui` feature, `uishell::UiShell`
//! implements them over `dphone-ui`.

pub mod engine;
mod host;
pub mod install;
pub mod manager;
pub mod manifest;
pub mod package;
pub mod services;
pub mod view;

#[cfg(feature = "test-util")]
pub mod testutil;
#[cfg(feature = "ui")]
pub mod uishell;

pub use engine::{AppEngine, EngineError};
pub use install::{InstallError, Installer};
pub use manager::{AppManager, ManagerConfig};
pub use manifest::{Capability, Manifest, ManifestError};
pub use services::{
    AppAccount, Completion, ConfigSource, HttpClient, HttpReply, HttpRequest, PbxRequest,
    PbxRequests, Services,
};
pub use view::{
    Align, AppWindow, Font, InputMode, ListItem, Metrics, Rect, Shell, Skin, Softkey, WidgetKind,
    WindowEvent, WindowKind,
};
