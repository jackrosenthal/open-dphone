//! The configuration store: loads the layers, merges them in the stock
//! order, hands out typed generations, and applies changes by writing the
//! right layer file.

use std::fmt;
use std::io;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::merge::{self, NetContext};
use crate::model::{BASE_CONFIG, Model};
use crate::settings::{OpenDeskPhoneSettings, SettingsDocument};
use crate::nvdata::{Nvdata, WEB_USER, htdigest_ha1, WEB_REALM};
use crate::view::{BlfItem, Config, parse_blf_items};
use crate::xml::{Element, Node};

/// The merge layers, in merge order.  The first ten are the stock layers in
/// the stock order.  Settings only Open Desk Phone has are not a layer: see
/// [`crate::settings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Compiled-in model-independent defaults (stock: `/etc/base-config.xml`).
    Base,
    /// Compiled-in model restrictions (stock: `/etc/factory-config.xml`).
    Factory,
    /// `/nvdata/g729-legacy-support.xml`
    G729Legacy,
    /// `/nvdata/g729-config.xml`
    G729Config,
    /// `/nvdata/netselect.xml`
    NetSelect,
    /// `/nvdata/configurator.xml`
    Configurator,
    /// `/nvdata/fetched-config.xml`
    Fetched,
    /// `/nvdata/ui-config.xml`
    Ui,
    /// `<tmp>/lldp.xml` (volatile)
    Lldp,
    /// `/nvdata/hack.xml`
    Hack,
}

impl Layer {
    /// All layers in merge order.
    pub const ORDER: [Layer; 10] = [
        Layer::Base,
        Layer::Factory,
        Layer::G729Legacy,
        Layer::G729Config,
        Layer::NetSelect,
        Layer::Configurator,
        Layer::Fetched,
        Layer::Ui,
        Layer::Lldp,
        Layer::Hack,
    ];

    /// Path relative to the nvdata root, for the layers stored there.
    pub fn nvdata_path(self) -> Option<&'static str> {
        Some(match self {
            Layer::G729Legacy => "g729-legacy-support.xml",
            Layer::G729Config => "g729-config.xml",
            Layer::NetSelect => "netselect.xml",
            Layer::Configurator => "configurator.xml",
            Layer::Fetched => "fetched-config.xml",
            Layer::Ui => "ui-config.xml",
            Layer::Hack => "hack.xml",
            Layer::Base | Layer::Factory | Layer::Lldp => return None,
        })
    }

    /// Whether this crate may write the layer.  The G.729 files and
    /// `hack.xml` belong to stock and are only read.
    pub fn writable(self) -> bool {
        matches!(
            self,
            Layer::NetSelect | Layer::Configurator | Layer::Fetched | Layer::Ui | Layer::Lldp
        )
    }
}

/// Prefixes the generic set path refuses: the retired `open_*` IDs, and the
/// `opendeskphone_*` IDs a provisioning server uses for Open Desk Phone's own
/// settings.  Locally those are changed in the settings file, never in
/// `ui-config.xml`.
const NOT_STOCK_PREFIXES: [&str; 2] = ["open_", "opendeskphone_"];
/// The factory default web password (stock ships `admin` / `789` in its
/// default digest file and as the base `login_password`).
pub const DEFAULT_WEB_PASSWORD: &str = "789";

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    /// The document handed in is not well-formed XML.
    Parse(String),
    /// The setting is frozen by an earlier layer; the change was not saved.
    Frozen(String),
    /// The layer is not one this crate writes.
    ReadOnlyLayer(Layer),
    /// Not a stock setting ID.  Open Desk Phone's own settings are changed with
    /// [`ConfigStore::update_opendeskphone_settings`].
    NotAStockSetting(String),
    /// More accounts than the model allows.
    TooManyAccounts { max: usize, given: usize },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "{e}"),
            ConfigError::Parse(e) => write!(f, "{e}"),
            ConfigError::Frozen(id) => write!(f, "setting {id} is frozen and cannot be changed"),
            ConfigError::NotAStockSetting(id) => {
                write!(f, "{id} is not a stock setting; use the Open Desk Phone settings instead")
            }
            ConfigError::ReadOnlyLayer(l) => write!(f, "layer {l:?} is read-only"),
            ConfigError::TooManyAccounts { max, given } => {
                write!(f, "{given} accounts given, but this model allows {max}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(e: io::Error) -> Self {
        ConfigError::Io(e)
    }
}

/// The configuration store.  Not internally synchronized: the owner (the
/// state store) serializes changes and shares the `Arc<Config>` it returns.
pub struct ConfigStore {
    nv: Nvdata,
    model: Model,
    tmp_dir: PathBuf,
    net: NetContext,
    current: Arc<Config>,
    warnings: Vec<String>,
    /// `/nvdata/opendeskphone/settings.txtpb` as last read or written.
    settings: SettingsDocument,
    /// Never rewrite the merged `/nvdata/config.xml` (see `open_read_only`).
    keep_merged_file: bool,
}

impl ConfigStore {
    /// Open the store and merge.  `tmp_dir` is where the volatile LLDP layer
    /// lives (`/tmp` on the phone).  Writes `/nvdata/config.xml` if the
    /// merged result differs from what is there.
    pub fn open(nv: Nvdata, model: Model, tmp_dir: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        Self::open_with(nv, model, tmp_dir, false)
    }

    /// Like [`ConfigStore::open`], but never rewrites the merged
    /// `/nvdata/config.xml`: for images that must leave stock-owned files
    /// exactly as they found them.  Explicit writes (`set`, `write_layer`)
    /// still write; the caller refuses those itself.
    pub fn open_read_only(nv: Nvdata, model: Model, tmp_dir: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        Self::open_with(nv, model, tmp_dir, true)
    }

    fn open_with(nv: Nvdata, model: Model, tmp_dir: impl Into<PathBuf>, keep_merged_file: bool) -> Result<Self, ConfigError> {
        let mut store = ConfigStore {
            keep_merged_file,
            nv,
            model,
            tmp_dir: tmp_dir.into(),
            net: NetContext::default(),
            current: Arc::new(Config::from_merged(Element::new("config"), model, None)),
            warnings: Vec::new(),
            settings: SettingsDocument::default(),
        };
        store.reload()?;
        Ok(store)
    }

    pub fn nvdata(&self) -> &Nvdata {
        &self.nv
    }

    pub fn model(&self) -> Model {
        self.model
    }

    /// The current merged generation.
    pub fn config(&self) -> Arc<Config> {
        self.current.clone()
    }

    /// Layers that existed but could not be parsed in the last merge (they
    /// were skipped, as the stock merge skips them).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Tell the merge the phone's current IPv4 address (it selects the
    /// network profile and filters `network=` elements), then re-merge.
    pub fn set_ip(&mut self, ip: Option<Ipv4Addr>) -> Result<Arc<Config>, ConfigError> {
        if self.net.ip != ip {
            self.net.ip = ip;
            self.reload()?;
        }
        Ok(self.config())
    }

    fn lldp_path(&self) -> PathBuf {
        self.tmp_dir.join("lldp.xml")
    }

    /// Read one layer; `Ok(None)` when it does not exist.
    pub fn layer(&self, layer: Layer) -> Result<Option<Element>, ConfigError> {
        let parse = |t: &str| Element::parse(t).map_err(|e| ConfigError::Parse(format!("{layer:?}: {e}")));
        match layer {
            Layer::Base => parse(BASE_CONFIG).map(Some),
            Layer::Factory => parse(self.model.factory_config()).map(Some),
            Layer::Lldp => match std::fs::read_to_string(self.lldp_path()) {
                Ok(t) => parse(&t).map(Some),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
            _ => match self.nv.read_string(layer.nvdata_path().unwrap())? {
                Some(t) => parse(&t).map(Some),
                None => Ok(None),
            },
        }
    }

    /// Re-read every layer and the settings file, and merge.  Returns the
    /// new generation, or the current one if nothing changed.
    pub fn reload(&mut self) -> Result<Arc<Config>, ConfigError> {
        let mut warnings = Vec::new();
        let mut acc = Element::new("config");
        for layer in Layer::ORDER {
            match self.layer(layer) {
                Ok(Some(e)) => merge::merge_layer(&mut acc, e, &self.net),
                Ok(None) => {}
                Err(ConfigError::Parse(e)) => warnings.push(e),
                Err(e) => return Err(e),
            }
        }
        self.warnings = warnings;
        // /nvdata/config.xml holds exactly the merge of the ten stock layers.
        let stock_doc = acc.to_document();
        if !self.keep_merged_file && self.nv.read_string("config.xml")?.as_deref() != Some(stock_doc.as_str()) {
            self.nv.write("config.xml", stock_doc.as_bytes())?;
        }
        self.settings = SettingsDocument::load(&self.nv)?;
        self.mirror_provisioned_ssh_keys(&acc)?;
        if acc != self.current.root
            || !self.settings.settings.same_as(&self.current.settings)
            || self.current.generation == 0
        {
            let selected = merge::selected_network(&acc, &self.net);
            self.current = Arc::new(Config {
                root: acc,
                generation: self.current.generation + 1,
                model: self.model,
                selected_network: selected,
                settings: self.settings.settings.clone(),
            });
        }
        Ok(self.config())
    }

    /// Keep `/nvdata/opendeskphone/provisioned_authorized_keys` equal to the
    /// keys in the merged configuration, so the SSH server can read them next
    /// to the locally added ones.  The file is derived state: local keys are
    /// never in it, and a key the server stops sending is gone after the next
    /// merge.
    fn mirror_provisioned_ssh_keys(&self, merged: &Element) -> Result<(), ConfigError> {
        let keys = Config::from_merged(merged.clone(), self.model, None).provisioned_ssh_keys();
        let want = if keys.is_empty() { None } else { Some(keys.join("\n") + "\n") };
        let rel = crate::settings::PROVISIONED_SSH_KEYS_FILE;
        if self.nv.read_string(rel)? != want {
            match want {
                Some(text) => self.nv.write(rel, text.as_bytes())?,
                None => self.nv.remove(rel)?,
            }
        }
        Ok(())
    }

    /// Every SSH public key to accept: the ones added on the phone
    /// (`/nvdata/opendeskphone/authorized_keys`) followed by the provisioned
    /// ones, without duplicates.
    pub fn ssh_authorized_keys(&self) -> Result<Vec<String>, ConfigError> {
        let mut keys = self.nv.authorized_keys()?;
        for k in self.current.provisioned_ssh_keys() {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        Ok(keys)
    }

    // ---- Open Desk Phone's own settings ------------------------------------

    /// The settings only Open Desk Phone has
    /// (`/nvdata/opendeskphone/settings.txtpb`).  The same value is on every
    /// generation: [`Config::opendeskphone_settings`].
    pub fn opendeskphone_settings(&self) -> &OpenDeskPhoneSettings {
        &self.settings.settings
    }

    /// Change Open Desk Phone's own settings: `change` edits the message (set a
    /// field to `Some(..)`, or to `None` to return it to its default), the
    /// file is written atomically, and a new generation is returned.  Fields
    /// in the file that this firmware does not know are kept.  Nothing is
    /// written when `change` changes nothing.
    pub fn update_opendeskphone_settings(
        &mut self,
        change: impl FnOnce(&mut OpenDeskPhoneSettings),
    ) -> Result<Arc<Config>, ConfigError> {
        let mut doc = SettingsDocument::load(&self.nv)?;
        let before = doc.settings.clone();
        change(&mut doc.settings);
        if !doc.settings.same_as(&before) {
            doc.save(&self.nv)?;
        }
        self.reload()
    }

    /// Replace a whole layer and re-merge.
    pub fn write_layer(&mut self, layer: Layer, root: &Element) -> Result<Arc<Config>, ConfigError> {
        if !layer.writable() {
            return Err(ConfigError::ReadOnlyLayer(layer));
        }
        match layer {
            Layer::Lldp => crate::nvdata::write_atomic(&self.lldp_path(), root.to_document().as_bytes())?,
            _ => self.nv.write_xml(layer.nvdata_path().unwrap(), root)?,
        }
        self.reload()
    }

    /// Remove a layer file and re-merge.
    pub fn remove_layer(&mut self, layer: Layer) -> Result<Arc<Config>, ConfigError> {
        if !layer.writable() {
            return Err(ConfigError::ReadOnlyLayer(layer));
        }
        match layer {
            Layer::Lldp => match std::fs::remove_file(self.lldp_path()) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            },
            _ => self.nv.remove(layer.nvdata_path().unwrap())?,
        }
        self.reload()
    }

    /// Change stock settings locally (phone menu, web UI, `dphone config
    /// set`).  Each is saved to the UI layer, as on stock, and the
    /// configuration is re-merged.  Fails without saving anything if one of
    /// them is frozen by an earlier layer.  Open Desk Phone's own settings are
    /// not setting IDs; see [`ConfigStore::update_opendeskphone_settings`].
    pub fn set_many(&mut self, changes: &[(&str, &str)]) -> Result<Arc<Config>, ConfigError> {
        for (id, _) in changes {
            if NOT_STOCK_PREFIXES.iter().any(|p| id.starts_with(p)) {
                return Err(ConfigError::NotAStockSetting((*id).to_owned()));
            }
            if self.frozen_below_local(id)? {
                return Err(ConfigError::Frozen((*id).to_owned()));
            }
        }
        if !changes.is_empty() {
            let mut root = self.layer_or_empty(Layer::Ui);
            for (id, value) in changes {
                match root.child_with_id_mut("setting", id) {
                    Some(e) => e.set_attr("value", value),
                    None => root.children.push(Node::Element(
                        Element::new("setting").with_attr("id", id).with_attr("value", value),
                    )),
                }
            }
            self.nv.write_xml(Layer::Ui.nvdata_path().unwrap(), &root)?;
        }
        self.reload()
    }

    /// Change one setting; see [`ConfigStore::set_many`].
    pub fn set(&mut self, id: &str, value: &str) -> Result<Arc<Config>, ConfigError> {
        self.set_many(&[(id, value)])
    }

    /// Drop a local override so the provisioned or default value shows again.
    pub fn unset(&mut self, id: &str) -> Result<Arc<Config>, ConfigError> {
        let layer = Layer::Ui;
        if let Some(mut root) = self.layer(layer).ok().flatten() {
            root.remove_children(|e| e.name == "setting" && e.attr("id") == Some(id));
            self.nv.write_xml(layer.nvdata_path().unwrap(), &root)?;
        }
        self.reload()
    }

    /// Value of a setting in the merged configuration.
    pub fn get(&self, id: &str) -> Option<String> {
        self.current.setting(id).map(str::to_owned)
    }

    fn layer_or_empty(&self, layer: Layer) -> Element {
        // A layer that does not parse is replaced, not appended to.
        self.layer(layer).ok().flatten().unwrap_or_else(|| Element::new("config"))
    }

    /// True if a layer merged before the local layers freezes the setting.
    fn frozen_below_local(&self, id: &str) -> Result<bool, ConfigError> {
        let mut acc = Element::new("config");
        for layer in Layer::ORDER {
            if layer == Layer::Ui {
                break;
            }
            if let Ok(Some(e)) = self.layer(layer) {
                merge::merge_layer(&mut acc, e, &self.net);
            }
        }
        Ok(acc.child_with_id("setting", id).and_then(|e| e.attr("conflict")) == Some("freeze"))
    }

    /// Replace the locally configured accounts (the UI layer's `<accounts>`
    /// collection, which replaces any provisioned collection, as on stock).
    /// Each element is an `<account>` in the stock shape.
    pub fn set_local_accounts(&mut self, accounts: &[Element]) -> Result<Arc<Config>, ConfigError> {
        let max = self.model.max_accounts();
        if accounts.len() > max {
            return Err(ConfigError::TooManyAccounts { max, given: accounts.len() });
        }
        let mut root = self.layer_or_empty(Layer::Ui);
        root.remove_children(|e| e.name == "accounts");
        let mut list = Element::new("accounts");
        list.children.extend(accounts.iter().cloned().map(Node::Element));
        root.children.push(Node::Element(list));
        self.write_layer(Layer::Ui, &root)
    }

    /// Remove the local accounts override so provisioned accounts apply.
    pub fn clear_local_accounts(&mut self) -> Result<Arc<Config>, ConfigError> {
        let mut root = self.layer_or_empty(Layer::Ui);
        root.remove_children(|e| e.name == "accounts");
        self.write_layer(Layer::Ui, &root)
    }

    /// Store a configuration fetched from the server (DPMA or URL) and
    /// re-merge.  The text must be well-formed; it is saved re-serialized in
    /// the stock layout.
    pub fn apply_fetched(&mut self, xml: &str) -> Result<Arc<Config>, ConfigError> {
        let root = Element::parse(xml).map_err(|e| ConfigError::Parse(e.to_string()))?;
        self.write_layer(Layer::Fetched, &root)
    }

    /// Choose a network profile explicitly, or `None` for automatic
    /// selection by CIDR (stock removes `netselect.xml` for that).
    pub fn select_network(&mut self, id: Option<&str>) -> Result<Arc<Config>, ConfigError> {
        match id {
            Some(id) => {
                let root = Element::new("config").with_child(Element::new("netselect").with_attr("id", id));
                self.write_layer(Layer::NetSelect, &root)
            }
            None => self.remove_layer(Layer::NetSelect),
        }
    }

    /// Settings learned from LLDP-MED (volatile layer 9), or `None` to clear.
    pub fn set_lldp(&mut self, settings: Option<&[(&str, &str)]>) -> Result<Arc<Config>, ConfigError> {
        match settings {
            Some(settings) => {
                let mut root = Element::new("config");
                for (id, value) in settings {
                    root.children.push(Node::Element(
                        Element::new("setting").with_attr("id", id).with_attr("value", value),
                    ));
                }
                self.write_layer(Layer::Lldp, &root)
            }
            None => self.remove_layer(Layer::Lldp),
        }
    }

    /// Stock's "reset UI settings": rebuild the UI layer keeping only the
    /// network settings, then re-merge.
    pub fn reset_ui_layer(&mut self) -> Result<Arc<Config>, ConfigError> {
        let mut root = self.layer_or_empty(Layer::Ui);
        root.children.retain(|n| match n {
            Node::Element(e) => {
                e.name == "setting" && e.attr("id").is_some_and(|id| id.starts_with("network"))
            }
            _ => false,
        });
        self.write_layer(Layer::Ui, &root)
    }

    // ---- web login ---------------------------------------------------------

    /// Check a web login.  The digest file is authoritative; if it has no
    /// entry for the user, `admin` is checked against `login_password`,
    /// and against the factory default (`789`) when that is not set either.
    pub fn verify_web_password(&self, user: &str, password: &str) -> Result<bool, ConfigError> {
        if let Some(ok) = self.nv.verify_web_password(user, password)? {
            return Ok(ok);
        }
        Ok(user == WEB_USER && self.fallback_web_password() == password)
    }

    /// HA1 (`MD5(user:realm:password)`) for HTTP digest authentication.
    pub fn web_ha1(&self, user: &str) -> Result<Option<String>, ConfigError> {
        if let Some(h) = self.nv.web_ha1(user)? {
            return Ok(Some(h));
        }
        Ok((user == WEB_USER).then(|| htdigest_ha1(user, WEB_REALM, self.fallback_web_password())))
    }

    /// The admin password when there is no digest file: `login_password`,
    /// or the factory default when that is absent or empty.
    fn fallback_web_password(&self) -> &str {
        self.current.setting("login_password").filter(|p| !p.is_empty()).unwrap_or(DEFAULT_WEB_PASSWORD)
    }

    /// Change the admin password: rewrites the digest file in stock's format
    /// and saves `login_password` in the UI layer, which is where stock
    /// regenerates the digest file from.
    pub fn set_web_password(&mut self, password: &str) -> Result<Arc<Config>, ConfigError> {
        self.nv.set_web_password(WEB_USER, password)?;
        self.set("login_password", password)
    }

    // ---- BLF ---------------------------------------------------------------

    /// BLF items: the local `/nvdata/blf_items.xml` if present, else the
    /// merged configuration's.
    pub fn blf_items(&self) -> Result<Vec<BlfItem>, ConfigError> {
        Ok(Self::blf_items_from(&self.nv, &self.current))
    }

    /// The same choice for a caller that holds a merged configuration and
    /// the nvdata handle rather than the store: `/nvdata/blf_items.xml`
    /// when it exists and parses, else `config`'s items.
    pub fn blf_items_from(nv: &Nvdata, config: &Config) -> Vec<BlfItem> {
        match nv.blf_items() {
            Ok(Some(doc)) => parse_blf_items(&doc),
            Ok(None) | Err(_) => config.blf_items(),
        }
    }

    // ---- factory reset -----------------------------------------------------

    /// Run a requested factory reset (call early at boot, before `open`):
    /// returns whether one was pending.
    pub fn run_pending_factory_reset(nv: &Nvdata, extra_keep: &[&str]) -> io::Result<bool> {
        if !nv.factory_reset_requested() {
            return Ok(false);
        }
        nv.factory_reset(extra_keep)?;
        nv.ensure_layout()?;
        Ok(true)
    }
}
