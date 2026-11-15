// Display names for values the phone reports as identifiers.  Every name is
// written out here; nothing is derived from an identifier by changing its
// case, which would turn SSH into "Ssh".  A value with no entry is shown as
// the raw identifier in the monospace font, so a missing entry is visible.

import type { ReactNode } from "react";
import { UpdatePhase } from "../gen/dphone/v1/firmware_pb";
import { NetPhase } from "../gen/dphone/v1/network_pb";
import { ProvisionPhase } from "../gen/dphone/v1/provisioning_pb";
import { SystemPhase } from "../gen/dphone/v1/status_pb";

type Names = Record<string | number, string>;

/// The display name of `value`, or the raw identifier in monospace.
export function label(names: Names, value: string | number | undefined, raw?: string): ReactNode {
  if (value === undefined || value === "") return "";
  return names[value] ?? <code className="font-mono text-[0.9em]">{raw ?? String(value)}</code>;
}

export const subsystemNames: Names = {
  core: "Core",
  config: "Configuration",
  net: "Network",
  sip: "SIP",
  media: "Media",
  dpma: "DPMA",
  ui: "Phone UI",
  apps: "Apps",
  web: "Web",
  ssh: "SSH",
  update: "Firmware",
};

export const systemPhaseNames: Names = {
  [SystemPhase.STARTING]: "Starting",
  [SystemPhase.NETWORK_WAIT]: "Waiting for network",
  [SystemPhase.PROVISIONING]: "Provisioning",
  [SystemPhase.RUNNING]: "Running",
  [SystemPhase.RECOVERY]: "Recovery",
  [SystemPhase.SHUTTING_DOWN]: "Shutting down",
};

export const netPhaseNames: Names = {
  [NetPhase.DOWN]: "Down",
  [NetPhase.CONFIGURING]: "Configuring",
  [NetPhase.UP]: "Up",
  [NetPhase.FAILED]: "Failed",
};

export const provisionPhaseNames: Names = {
  [ProvisionPhase.IDLE]: "Idle",
  [ProvisionPhase.DISCOVERING]: "Discovering",
  [ProvisionPhase.AWAITING_SERVER]: "Waiting for server",
  [ProvisionPhase.HANDSHAKING]: "Connecting",
  [ProvisionPhase.AWAITING_USER]: "Waiting for user",
  [ProvisionPhase.FETCHING_CONFIG]: "Downloading configuration",
  [ProvisionPhase.APPLYING]: "Applying",
  [ProvisionPhase.PROVISIONED]: "Provisioned",
  [ProvisionPhase.STANDALONE]: "Standalone",
  [ProvisionPhase.FAILED]: "Failed",
};

export const updatePhaseNames: Names = {
  [UpdatePhase.IDLE]: "Idle",
  [UpdatePhase.RECEIVING]: "Uploading",
  [UpdatePhase.CHECKING]: "Validating",
  [UpdatePhase.ERASING]: "Erasing",
  [UpdatePhase.WRITING_KERNEL]: "Writing kernel",
  [UpdatePhase.WRITING_ROOTFS]: "Writing root filesystem",
  [UpdatePhase.VERIFYING]: "Verifying",
  [UpdatePhase.COMMITTING]: "Finishing",
  [UpdatePhase.COMPLETE]: "Complete",
  [UpdatePhase.FAILED]: "Failed",
};

/// Where the IPv4 address came from.
export const addressSourceNames: Names = { dhcp: "DHCP", static: "Static" };
/// Where the voice VLAN came from.
export const vlanSourceNames: Names = { config: "Manual", lldp: "LLDP-MED", dhcp: "DHCP" };
/// How a provisioning server was found.
export const discoverySourceNames: Names = { saved: "Saved", dhcp: "DHCP option 66", mdns: "mDNS", manual: "Manual" };
/// Codecs, which the phone reports by their SDP encoding names.
export const codecNames: Names = {
  PCMU: "G.711 µ-law",
  PCMA: "G.711 A-law",
  G722: "G.722",
  "G726-32": "G.726",
  G729: "G.729",
  iLBC: "iLBC",
  opus: "Opus",
  L16: "L16",
  "L16-256": "L16 wideband",
};
