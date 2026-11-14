import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useState } from "react";
import { Alert, Badge, Button, Card, Field, Input, KeyValue, PageHeader, Select, Spinner, Toggle } from "../components/ui";
import { NetPhase, NetworkService, type NetworkSettings } from "../gen/dphone/v1/network_pb";
import { enumLabel } from "../lib/format";
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
  const sync = useMutation(NetworkService.method.syncTime, { onError: setError });

  if (q.error) return <ErrorNote error={q.error} />;
  if (!q.data) return <Spinner />;
  const net = status?.network ?? q.data.status!;
  const s = draft ?? q.data.settings!;
  const set = <K extends keyof NetworkSettings>(k: K, v: NetworkSettings[K]) => {
    setSaved(false);
    setDraft({ ...s, [k]: v });
  };
  const text = (k: "staticIp" | "subnetMask" | "gateway" | "primaryDns" | "secondaryDns" | "domain" | "ntpServer" | "timeZone", label: string, disabled = false) => (
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
      <PageHeader
        title="Network"
        actions={
          <>
            <Button onClick={() => renew.mutate({})} busy={renew.isPending}>
              Renew DHCP lease
            </Button>
            <Button onClick={() => sync.mutate({})} busy={sync.isPending}>
              Synchronize time
            </Button>
          </>
        }
      />
      <div className="space-y-5">
        <ErrorNote error={error} />
        {saved && <Alert tone="green" title="Saved and applied." />}
        <div className="grid gap-5 lg:grid-cols-2">
          <Card title="Status" actions={<Badge tone={net.phase === NetPhase.UP ? "green" : net.phase === NetPhase.FAILED ? "red" : "amber"}>{enumLabel(NetPhase[net.phase], "")}</Badge>}>
            <KeyValue
              rows={[
                ["Address", net.ipv4Address ? `${net.ipv4Address}/${net.ipv4PrefixLen}` : ""],
                ["Source", net.ipv4Source],
                ["Gateway", net.ipv4Gateway],
                ["DHCP server", net.dhcpServer],
                ["Lease", net.leaseSecs ? `${net.leaseSecs} s` : ""],
                ["IPv6", net.ipv6Addresses.join(", ")],
                ["DNS", net.dns.join(", ")],
                ["Domain", net.domain],
                ["NTP", `${net.ntpServers.join(", ")}${net.ntpSynced ? " (synchronized)" : ""}`],
                ["Option 66", net.dhcpTftpServer],
                ["Profile", net.profile],
                ["Detail", net.detail],
              ]}
            />
          </Card>
          <Card title="Ports and VLAN">
            <KeyValue
              rows={[
                ["MAC address", net.mac],
                ["LAN port", net.lan?.up ? `${net.lan.speed} Mb/s, ${net.lan.fullDuplex ? "full" : "half"} duplex` : "No link"],
                ["PC port", !net.pcPortEnabled ? "Disabled" : net.pc?.up ? `${net.pc.speed} Mb/s, ${net.pc.fullDuplex ? "full" : "half"} duplex` : "No link"],
                ["Voice VLAN", net.voiceVlan !== undefined ? `${net.voiceVlan}, priority ${net.vlanPriority} (from ${net.vlanSource})` : "Untagged"],
                ["PC port VLAN", net.pcVlan !== undefined ? String(net.pcVlan) : ""],
                ["DSCP", String(net.dscp)],
                ["LLDP neighbor", net.lldpPresent ? net.lldpSystemName : "None seen"],
                ["Neighbor port", [net.lldpPortId, net.lldpPortDescription].filter(Boolean).join(" · ")],
                ["Neighbor chassis", net.lldpChassisId],
              ]}
            />
          </Card>
        </div>

        <Card
          title="Settings"
          actions={
            <>
              {draft && <Button onClick={() => setDraft(undefined)}>Discard</Button>}
              <Button variant="primary" disabled={!draft} busy={save.isPending} onClick={() => save.mutate({ settings: s, apply: true })}>
                Save and apply
              </Button>
            </>
          }
        >
          <div className="space-y-6">
            {draft && !s.dhcp && <Alert>Changing the address disconnects this page; reopen it at the new address.</Alert>}
            <section className="space-y-4">
              <Toggle label="Get an address automatically (DHCP)" checked={s.dhcp} onChange={(v) => set("dhcp", v)} />
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
              <Field label="Voice VLAN discovery">
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
              {num("pcQos", "PC port priority", 7)}
              {num("sipQos", "SIP 802.1p priority", 7)}
              {num("rtpQos", "RTP 802.1p priority", 7)}
              <div className="flex items-end pb-2">
                <Toggle label="Pass 802.1X from the PC port" checked={s.dot1xPassthrough} onChange={(v) => set("dot1xPassthrough", v)} />
              </div>
            </section>
            <section className="grid gap-4 border-t border-slate-100 pt-5 dark:border-slate-800 sm:grid-cols-3">
              {text("ntpServer", "NTP server")}
              {text("timeZone", "Time zone")}
            </section>
          </div>
        </Card>
      </div>
    </>
  );
}
