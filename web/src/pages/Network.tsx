import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useState } from "react";
import { Alert, Badge, Button, Card, Field, Input, KeyValue, PageHeader, Select, Spinner, Toggle } from "../components/ui";
import { NetPhase, NetworkService, type NetworkSettings } from "../gen/dphone/v1/network_pb";
import { duration } from "../lib/format";
import { addressSourceNames, label, netPhaseNames, vlanSourceNames } from "../lib/labels";
import { useStatus } from "../lib/status";
import { ErrorNote } from "./common";

export function Network() {
  const { status } = useStatus();
  const q = useQuery(NetworkService.method.getNetwork, {});
  const [draft, setDraft] = useState<NetworkSettings>();
  const [error, setError] = useState<unknown>();
  const [saved, setSaved] = useState(false);
  const save = useMutation(NetworkService.method.setNetworkSettings, {
    onSuccess: () => {
      setDraft(undefined);
      setError(undefined);
      setSaved(true);
      q.refetch();
    },
    onError: setError,
  });
  const renew = useMutation(NetworkService.method.renewDhcp, { onError: setError });

  if (q.error) return <ErrorNote error={q.error} />;
  if (!q.data) return <Spinner />;
  const net = status?.network ?? q.data.status!;
  const s = draft ?? q.data.settings!;
  const set = <K extends keyof NetworkSettings>(k: K, v: NetworkSettings[K]) => {
    setSaved(false);
    setDraft({ ...s, [k]: v });
  };
  const text = (k: "staticIp" | "subnetMask" | "gateway" | "primaryDns" | "secondaryDns" | "domain", label: string, disabled = false) => (
    <Field label={label}>
      <Input value={s[k]} disabled={disabled} onChange={(e) => set(k, e.target.value)} />
    </Field>
  );
  const num = (k: "sipDscp" | "rtpDscp" | "sipQos" | "rtpQos" | "pcQos", label: string, max: number) => (
    <Field label={label}>
      <Input type="number" min={0} max={max} value={s[k]} onChange={(e) => set(k, Number(e.target.value))} />
    </Field>
  );
  const vlan = (k: "vlanId" | "pcVlanId", label: string, disabled = false) => (
    <Field label={label}>
      <Input type="number" min={1} max={4094} placeholder="None" disabled={disabled} value={s[k] ?? ""} onChange={(e) => set(k, e.target.value === "" ? undefined : Number(e.target.value))} />
    </Field>
  );

  return (
    <>
      <PageHeader title="Network" />
      <div className="space-y-5">
        <ErrorNote error={error} />
        {saved && <Alert tone="green" title="Saved." />}
        <div className="grid gap-5 lg:grid-cols-2">
          <Card
            title="Status"
            actions={
              <>
                <Badge tone={net.phase === NetPhase.UP ? "green" : net.phase === NetPhase.FAILED ? "red" : "amber"}>{label(netPhaseNames, net.phase, NetPhase[net.phase])}</Badge>
                {q.data.settings?.dhcp && (
                  <Button size="sm" onClick={() => renew.mutate({})} busy={renew.isPending}>
                    Renew DHCP
                  </Button>
                )}
              </>
            }
          >
            <KeyValue
              rows={[
                ["IP address", net.ipv4Address && <>{net.ipv4Address}/{net.ipv4PrefixLen}{net.ipv4Source && <> ({label(addressSourceNames, net.ipv4Source)})</>}</>],
                ["Gateway", net.ipv4Gateway],
                ["DHCP server", net.dhcpServer],
                ["Lease time", net.leaseSecs ? duration(net.leaseSecs) : ""],
                ["IPv6", net.ipv6Addresses.join(", ")],
                ["DNS", net.dns.join(", ")],
                ["Domain", net.domain],
                ["NTP", `${net.ntpServers.join(", ")}${net.ntpSynced ? " (synced)" : ""}`],
                ["DHCP option 66", net.dhcpTftpServer],
                ["Profile", net.profile],
                ["Details", net.detail],
              ]}
            />
          </Card>
          <Card title="Ports and VLANs">
            <KeyValue
              rows={[
                ["MAC address", net.mac],
                ["LAN port", net.lan?.up ? `${net.lan.speed} Mbps, ${net.lan.fullDuplex ? "full" : "half"} duplex` : "No link"],
                ["PC port", !net.pcPortEnabled ? "Disabled" : net.pc?.up ? `${net.pc.speed} Mbps, ${net.pc.fullDuplex ? "full" : "half"} duplex` : "No link"],
                ["Voice VLAN", net.voiceVlan !== undefined ? <>{net.voiceVlan}, priority {net.vlanPriority}{net.vlanSource && <> ({label(vlanSourceNames, net.vlanSource)})</>}</> : "None"],
                ["PC port VLAN", net.pcVlan !== undefined ? String(net.pcVlan) : ""],
                ["DSCP", String(net.dscp)],
                ["Switch", net.lldpPresent ? net.lldpSystemName : ""],
                ["Switch port", [net.lldpPortId, net.lldpPortDescription].filter(Boolean).join(" · ")],
                ["Switch chassis ID", net.lldpChassisId],
              ]}
            />
          </Card>
        </div>

        <Card
          title="Settings"
          actions={
            <>
              {draft && <Button onClick={() => setDraft(undefined)}>Discard</Button>}
              <Button variant="primary" disabled={!draft} busy={save.isPending} onClick={() => save.mutate({ settings: { ...s, ntpServer: "", timeZone: "" }, apply: true })}>
                Save
              </Button>
            </>
          }
        >
          <div className="space-y-6">
            <section className="space-y-4">
              <Toggle label="DHCP" checked={s.dhcp} onChange={(v) => set("dhcp", v)} />
              <div className="grid gap-4 sm:grid-cols-3">
                {text("staticIp", "IP address", s.dhcp)}
                {text("subnetMask", "Subnet mask", s.dhcp)}
                {text("gateway", "Gateway", s.dhcp)}
                {text("primaryDns", "Primary DNS", s.dhcp)}
                {text("secondaryDns", "Secondary DNS", s.dhcp)}
                {text("domain", "Domain")}
              </div>
              <Toggle label="IPv6" checked={s.ipv6Enabled} onChange={(v) => set("ipv6Enabled", v)} />
            </section>
            <section className="grid gap-4 border-t border-slate-100 pt-5 dark:border-slate-800 sm:grid-cols-3">
              <Field label="VLAN discovery">
                <Select value={s.vlanDiscoveryMode.toUpperCase()} onChange={(e) => set("vlanDiscoveryMode", e.target.value)}>
                  <option value="LLDP">LLDP-MED</option>
                  <option value="MANUAL">Manual</option>
                  <option value="NONE">None</option>
                </Select>
              </Field>
              {vlan("vlanId", "Voice VLAN ID", s.vlanDiscoveryMode.toUpperCase() !== "MANUAL")}
              {vlan("pcVlanId", "PC port VLAN ID")}
              {num("sipDscp", "SIP DSCP", 63)}
              {num("rtpDscp", "RTP DSCP", 63)}
              {num("pcQos", "PC port 802.1p priority", 7)}
              {num("sipQos", "SIP 802.1p priority", 7)}
              {num("rtpQos", "RTP 802.1p priority", 7)}
              <div className="flex items-end pb-2">
                <Toggle label="802.1X passthrough" checked={s.dot1xPassthrough} onChange={(v) => set("dot1xPassthrough", v)} />
              </div>
            </section>
          </div>
        </Card>
      </div>
    </>
  );
}
