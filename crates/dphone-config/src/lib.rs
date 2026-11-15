//! Open DPhone configuration: the `nvdata` file layer, the stock-compatible
//! ten-layer XML merge, and a typed view of the merged result.
//!
//! Typical use:
//!
//! ```no_run
//! use dphone_config::{ConfigStore, Model, Nvdata};
//!
//! let nv = Nvdata::new("/nvdata");
//! ConfigStore::run_pending_factory_reset(&nv, &[])?;
//! nv.ensure_layout()?;
//! let mut store = ConfigStore::open(nv, Model::D60, "/tmp")?;
//! let config = store.config();
//! for account in config.accounts() {
//!     println!("{} at {:?}", account.username, account.primary);
//! }
//! let config = store.set("ringer_volume", "7")?;
//! assert_eq!(config.setting("ringer_volume"), Some("7"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod contacts;
pub mod merge;
pub mod model;
pub mod nvdata;
pub mod palette;
pub mod settings;
pub mod store;
pub mod trust;
pub mod view;
pub mod xml;

pub use merge::NetContext;
pub use model::Model;
pub use nvdata::{parse_authorized_key, AuthorizedKey, EnetCfg, Nvdata, SavedTime};
pub use palette::PaletteFile;
pub use settings::{DphoneSettings, IdleFunction, IdleSoftkey, IdleSoftkeys, Resolved, SettingSource, SettingsDocument, Wallpaper, WallpaperChoice};
pub use store::{ConfigError, ConfigStore, Layer};
pub use view::Config;
pub use xml::{Element, Node};
