import { Link } from "@tanstack/react-router";
import { Badge, Card, Dot, Empty, KeyValue, PageHeader, Spinner } from "../components/ui";
import { CallDirection } from "../gen/dphone/v1/calls_pb";
import { NetPhase } from "../gen/dphone/v1/network_pb";
import { ProvisionPhase } from "../gen/dphone/v1/provisioning_pb";
import { HealthState, SystemPhase } from "../gen/dphone/v1/status_pb";
import { dateTime, duration, enumLabel } from "../lib/format";
import { useStatus } from "../lib/status";
import { RegistrationBadge, callStateLabel } from "./common";

const healthTone = {
  [HealthState.UNSPECIFIED]: "gray",
  [HealthState.STOPPED]: "gray",
  [HealthState.STARTING]: "blue",
  [HealthState.OK]: "green",
  [HealthState.DEGRADED]: "amber",
  [HealthState.FAILED]: "red",
} as const;

export function Dashboard() {
  const { status, elapsed } = useStatus();
  if (!status) {
    return (
      <div className="flex justify-center py-20 text-slate-400">
        <Spinner className="size-8" />
      </div>
    );
  }
  const sys = status.system!;
  const net = status.network!;
  const prov = status.provisioning!;
  const calls = status.calls!;
  const newMessages = calls.mwi.reduce((n, m) => n + m.newMessages, 0);

  return (
    <>
      <PageHeader
        title="Dashboard"
        description={`Digium ${sys.model} running Open DPhone ${sys.version}`}
        actions={
          <Badge tone={sys.phase === SystemPhase.RUNNING ? "green" : sys.phase === SystemPhase.RECOVERY ? "red" : "amber"}>
            {enumLabel(SystemPhase[sys.phase], "")}
          </Badge>
        }
      />
      <div className="grid gap-5 lg:grid-cols-2">
        <Card title="Lines" actions={<Link to="/accounts" className="text-xs font-medium text-brand-700 hover:underline dark:text-brand-300">Manage</Link>}>
          {status.accounts.length === 0 ? (
            <Empty>No accounts are configured.</Empty>
          ) : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {status.accounts.map((a) => (
                <li key={a.index} className="flex items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">
                      {a.label} {a.isDefault && <span className="text-xs font-normal text-slate-400">default</span>}
                    </p>
                    <p className="truncate text-xs text-slate-500 dark:text-slate-400">
                      {a.user}@{a.server}
                      {a.reason ? ` · ${a.reason}` : ""}
                      {a.retryInSecs ? ` · retry in ${a.retryInSecs} s` : ""}
                    </p>
                  </div>
                  <RegistrationBadge a={a} />
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Calls" actions={<Link to="/calls" className="text-xs font-medium text-brand-700 hover:underline dark:text-brand-300">Open</Link>}>
          {calls.calls.length === 0 ? (
            <Empty>No calls in progress.</Empty>
          ) : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {calls.calls.map((c) => (
                <li key={c.id} className="flex items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{c.remote?.name || c.remote?.number}</p>
                    <p className="text-xs text-slate-500 dark:text-slate-400">
                      {c.direction === CallDirection.INCOMING ? "Incoming" : "Outgoing"}
                      {c.codec ? ` · ${c.codec}` : ""}
                      {c.durationSecs !== undefined ? ` · ${duration(Number(c.durationSecs) + elapsed)}` : ""}
                    </p>
                  </div>
                  <Badge tone="blue">{callStateLabel(c.state)}</Badge>
                </li>
              ))}
            </ul>
          )}
          <div className="mt-4 flex flex-wrap gap-2 text-xs">
            {calls.features?.dnd && <Badge tone="red">Do not disturb</Badge>}
            {calls.features?.forwardAlways && <Badge tone="amber">Forwarding to {calls.features.forwardAlways}</Badge>}
            {newMessages > 0 && <Badge tone="blue">{newMessages} new voicemail</Badge>}
            {calls.missedUnseen > 0 && <Badge tone="amber">{calls.missedUnseen} missed</Badge>}
          </div>
        </Card>

        <Card title="Network" actions={<Badge tone={net.phase === NetPhase.UP ? "green" : net.phase === NetPhase.FAILED ? "red" : "amber"}>{enumLabel(NetPhase[net.phase], "")}</Badge>}>
          <KeyValue
            rows={[
              ["Address", net.ipv4Address ? `${net.ipv4Address}/${net.ipv4PrefixLen} (${net.ipv4Source || "none"})` : ""],
              ["Gateway", net.ipv4Gateway],
              ["DNS", net.dns.join(", ")],
              ["LAN port", net.lan?.up ? `${net.lan.speed} Mb/s ${net.lan.fullDuplex ? "full" : "half"} duplex` : "No link"],
              ["PC port", !net.pcPortEnabled ? "Disabled" : net.pc?.up ? `${net.pc.speed} Mb/s` : "No link"],
              ["Voice VLAN", net.voiceVlan !== undefined ? `${net.voiceVlan} (${net.vlanSource})` : "Untagged"],
              ["Switch", net.lldpPresent ? `${net.lldpSystemName} ${net.lldpPortId}` : ""],
              ["MAC", net.mac],
            ]}
          />
        </Card>

        <Card title="Provisioning" actions={<Badge tone={prov.phase === ProvisionPhase.PROVISIONED ? "green" : prov.phase === ProvisionPhase.FAILED ? "red" : "gray"}>{enumLabel(ProvisionPhase[prov.phase], "")}</Badge>}>
          <KeyValue
            rows={[
              ["Server", prov.server ? `${prov.server.name} (${prov.server.source})` : ""],
              ["Server software", prov.serverUserAgent],
              ["Assigned to", prov.assignedUser],
              ["Session", prov.attached ? "Attached" : "Not attached"],
              ["Last success", dateTime(prov.lastSuccessUnix)],
              ["Failure", prov.failureReason],
            ]}
          />
        </Card>

        <Card title="System">
          <KeyValue
            rows={[
              ["Host name", sys.hostname],
              ["Uptime", duration(Number(sys.uptimeSecs) + elapsed)],
              ["Phone time", `${dateTime(Number(sys.unixTime) + elapsed)}${sys.timeSynced ? "" : " (not synchronized)"}`],
              ["Time zone", sys.timezone],
              ["Memory", `${Math.round(sys.memTotalKib / 1024)} MiB usable`],
              ["Last restart", sys.lastRebootReason],
              ["Firmware bank", status.firmware?.runningBank],
              ["User agent", sys.userAgent],
            ]}
          />
        </Card>

        <Card title="Subsystems" actions={status.healthy ? <Badge tone="green">Healthy</Badge> : <Badge tone="amber">Not yet healthy</Badge>}>
          <ul className="grid grid-cols-2 gap-x-6 gap-y-2 text-sm sm:grid-cols-3">
            {status.health.map((h) => (
              <li key={h.subsystem} className="flex items-center gap-2" title={h.detail}>
                <Dot tone={healthTone[h.state]} />
                <span className="capitalize">{h.subsystem}</span>
                {h.detail && <span className="truncate text-xs text-slate-400">{h.detail}</span>}
              </li>
            ))}
          </ul>
        </Card>
      </div>
    </>
  );
}
