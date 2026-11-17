#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// A temporary directory removed on drop.
pub struct TempDir(pub PathBuf);

static N: AtomicU32 = AtomicU32::new(0);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "dphone-config-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A provisioned configuration in the shape a DPMA server sends.
/// What a DPMA server adds to its configuration to provision Open Desk Phone's
/// own settings and SSH keys.
pub const DPHONE_EXTENSION: &str = r#"
  <setting id="dphone_ssh_enabled" value="1" conflict="freeze" />
  <setting id="dphone_ignore_provisioned_firmware" value="0" />
  <dphone_ssh_authorized_keys>
    <key id="1" value="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa alice@laptop" />
    <key id="2" value="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb bob@desk" />
  </dphone_ssh_authorized_keys>
"#;

/// [`FETCHED`] with [`DPHONE_EXTENSION`] added.
pub fn fetched_with_extension() -> String {
    FETCHED.replace("</config>", &format!("{DPHONE_EXTENSION}</config>"))
}

pub const FETCHED: &str = r#"<?xml version="1.0"?>
<config>
  <networks>
    <network id="office" display_name="Office" cidr="10.10.0.0/16" />
    <network id="any" display_name="Anywhere" cidr="0.0.0.0/0" />
  </networks>
  <setting id="ringer_volume" value="3" />
  <setting id="time_zone" value="America/Denver" />
  <setting id="log_server" value="10.10.0.5" network_id="office" />
  <setting id="enable_logging" value="1" />
  <setting id="active_ringtone" value="Chime" />
  <accounts>
    <account network_id="office" index="0" account_id="101" username="101" password="pw101"
             passcode="pw101" caller_id="Alice" number="101" display="101" status="1" type="sip"
             authname="101" line_label="Alice" register="1" visual_voicemail="1">
      <host_primary server="10.10.0.2" port="5061" transport="tls" media_encryption="sdes" reregister="120" />
      <host_alternate server="10.10.0.3" port="5060" transport="udp" />
      <setting id="parking_lot_extension" value="" />
    </account>
    <account network_id="any" index="0" account_id="101" username="101" password="pw101"
             passcode="pw101" number="101" status="1" type="sip" register="1">
      <host_primary server="pbx.example.com" port="5060" transport="udp" />
    </account>
    <account network_id="office" index="1" account_id="102" username="102" password="pw102" passcode="pw102">
      <host_primary server="10.10.0.2" port="5060" transport="tcp" />
    </account>
    <account network_id="office" index="2" account_id="103" username="103" password="pw103" passcode="pw103">
      <host_primary server="10.10.0.2" port="5060" transport="tcp" />
    </account>
  </accounts>
  <firmwares>
    <firmware model="D60" version="2_9_28" url="http://10.10.0.2/fw/2_9_28_D60_firmware.eff" network_id="office" />
    <firmware model="D40" version="2_9_28" url="http://10.10.0.2/fw/2_9_28_D40_firmware.eff" network_id="office" />
  </firmwares>
  <contacts url="http://10.10.0.2/contacts.xml" id="0" network_id="office" />
  <smart_blf>
    <blf_items url="http://10.10.0.2/blf.xml" network_id="office"/>
  </smart_blf>
  <ringtones>
    <tones><tone id="Custom" display="Custom" url="http://10.10.0.2/custom.raw" type="user" /></tones>
    <alerts><alert alert_info="intercom" ringtone_id="" ring_type="answer" /></alerts>
  </ringtones>
  <multicastpage>
    <listener id="1" name="All" addr="239.1.1.1" port="5000" priority="1" interrupt_calls="1" />
    <broadcast id="b1" name="Warehouse" addr="239.1.1.2" port="5002" codec="G722" />
    <broadcast id="b2" name="Office" addr="239.1.1.4" port="5006" />
  </multicastpage>
  <keymap>
    <state id="idle" conflict="replace">
      <softkeys><key id="0" action="menu"/><key id="1" action="show_application" label1="_VM"><arg>voicemail</arg></key></softkeys>
    </state>
  </keymap>
</config>
"#;
