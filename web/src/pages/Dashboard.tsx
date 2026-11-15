import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Badge, Card, Dot, Empty, KeyValue, PageHeader, Spinner } from "../components/ui";
import { CallDirection } from "../gen/dphone/v1/calls_pb";
import { NetPhase } from "../gen/dphone/v1/network_pb";
import { ProvisionPhase } from "../gen/dphone/v1/provisioning_pb";
import { HealthState, SystemPhase, type StatusSnapshot } from "../gen/dphone/v1/status_pb";
import { RegistrationState } from "../gen/dphone/v1/accounts_pb";
import { dateTime, duration } from "../lib/format";
import { addressSourceNames, codecNames, discoverySourceNames, label, netPhaseNames, provisionPhaseNames, subsystemNames, systemPhaseNames, vlanSourceNames } from "../lib/labels";
import { useStatus } from "../lib/status";
import { RegistrationBadge, callStateLabel } from "./common";
import { ExpansionCard } from "./Expansion";
import { Recovery } from "./Recovery";

const healthTone = {
  [HealthState.UNSPECIFIED]: "gray",
  [HealthState.STOPPED]: "gray",
  [HealthState.STARTING]: "blue",
  [HealthState.OK]: "green",
  [HealthState.DEGRADED]: "amber",
  [HealthState.FAILED]: "red",
} as const;

/// A reason as the firmware words it, in the wording of this interface.
/// A reason with no entry is shown as it is, capitalized.
const reasons: Record<string, string> = {
  "no listener": "Not listening",
  "not listening": "Not listening",
  "no configuration": "No configuration",
  "not responding": "Not responding",
  "no UI": "Phone UI not running",
  "no MAC address": "No MAC address",
  "no hardware layer": "No hardware layer",
  "manager task ended": "Network manager stopped",
  "the UI did not start": "Phone UI did not start",
  "the display does not answer": "Display not responding",
  "the app runtime thread panicked": "App runtime crashed",
};
const reason = (text: string) => reasons[text] ?? text.charAt(0).toUpperCase() + text.slice(1);

type Problem = { key: string; name: ReactNode; state: string; why: string; tone: "red" | "amber" };

/// Everything the phone reports as failed or degraded, each with its reason.
function problems(status: StatusSnapshot): Problem[] {
  const list: Problem[] = [];
  for (const h of status.health) {
    if (h.state !== HealthState.FAILED && h.state !== HealthState.DEGRADED) continue;
    const failed = h.state === HealthState.FAILED;
    list.push({ key: h.subsystem, name: label(subsystemNames, h.subsystem), state: failed ? "Failed" : "Degraded", why: reason(h.detail), tone: failed ? "red" : "amber" });
  }
  const net = status.network;
  if (net?.phase === NetPhase.FAILED && !list.some((p) => p.key === "net")) {
    list.push({ key: "network", name: "Network", state: "Failed", why: reason(net.detail), tone: "red" });
  }
  for (const a of status.accounts) {
    if (a.state !== RegistrationState.FAILED) continue;
    const why = [a.code ? String(a.code) : "", a.reason].filter(Boolean).join(" ");
    list.push({ key: `account${a.index}`, name: a.label, state: "Registration failed", why, tone: "red" });
  }
  const prov = status.provisioning;
  if (prov?.phase === ProvisionPhase.FAILED) {
    const why = `${reason(prov.failureReason)}${prov.failureCode !== undefined ? ` (code ${prov.failureCode})` : ""}`;
    list.push({ key: "provisioning", name: "Provisioning", state: "Failed", why, tone: "red" });
  }
  return list;
}

const quietState: Partial<Record<HealthState, string>> = {
  [HealthState.STOPPED]: "Stopped",
  [HealthState.STARTING]: "Starting",
  [HealthState.UNSPECIFIED]: "Unknown",
};

function StatusCard({ status }: { status: StatusSnapshot }) {
  const failing = problems(status);
  const rest = status.health.filter((h) => h.state !== HealthState.FAILED && h.state !== HealthState.DEGRADED);
  const warning = failing.length > 0 || !status.healthy;
  return (
    <Card title="Status" actions={warning ? <Badge tone="amber">Warning</Badge> : <Badge tone="green">OK</Badge>}>
      <div className="space-y-4">
        {failing.length > 0 && (
          <ul className="space-y-2">
            {failing.map((p) => (
              <li key={p.key} className={p.tone === "red" ? "rounded-lg bg-red-50 px-3 py-2 text-sm ring-1 ring-inset ring-red-200 dark:bg-red-950/40 dark:ring-red-900" : "rounded-lg bg-amber-50 px-3 py-2 text-sm ring-1 ring-inset ring-amber-200 dark:bg-amber-950/40 dark:ring-amber-900"}>
                <p className={p.tone === "red" ? "font-semibold text-red-900 dark:text-red-200" : "font-semibold text-amber-900 dark:text-amber-200"}>
                  {p.name}: {p.state}
                </p>
                {p.why && <p className="text-slate-700 dark:text-slate-300">{p.why}</p>}
              </li>
            ))}
          </ul>
        )}
        <ul className="grid grid-cols-2 gap-x-6 gap-y-2 text-sm sm:grid-cols-3">
          {rest.map((h) => (
            <li key={h.subsystem} className="flex items-center gap-2">
              <Dot tone={healthTone[h.state]} />
              <span>{label(subsystemNames, h.subsystem)}</span>
              {quietState[h.state] && <span className="text-xs text-slate-500 dark:text-slate-400">{quietState[h.state]}</span>}
            </li>
          ))}
        </ul>
      </div>
    </Card>
  );
}

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
  if (sys.phase === SystemPhase.RECOVERY) return <Recovery model={sys.model} />;
  const net = status.network!;
  const prov = status.provisioning!;
  const calls = status.calls!;
  const newMessages = calls.mwi.reduce((n, m) => n + m.newMessages, 0);

  return (
    <>
      <PageHeader
        title="Dashboard"
        actions={sys.phase !== SystemPhase.RUNNING && <Badge tone="amber">{label(systemPhaseNames, sys.phase, SystemPhase[sys.phase])}</Badge>}
      />
      <div className="grid gap-5 lg:grid-cols-2">
        <Card title="Accounts" actions={<Link to="/accounts" className="text-xs font-medium text-brand-700 hover:underline dark:text-brand-300">Manage</Link>}>
          {status.accounts.length === 0 ? (
            <Empty>No accounts</Empty>
          ) : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {status.accounts.map((a) => (
                <li key={a.index} className="flex items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">
                      {a.label} {a.isDefault && <span className="text-xs font-normal text-slate-400">Default</span>}
                    </p>
                    <p className="truncate text-xs text-slate-500 dark:text-slate-400">
                      {a.user}@{a.server}
                      {a.reason ? ` · ${a.reason}` : ""}
                      {a.retryInSecs ? ` · retrying in ${a.retryInSecs} s` : ""}
                    </p>
                  </div>
                  <RegistrationBadge a={a} />
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Calls" actions={<Link to="/calls" className="text-xs font-medium text-brand-700 hover:underline dark:text-brand-300">View</Link>}>
          {calls.calls.length === 0 ? (
            <Empty>No active calls</Empty>
          ) : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {calls.calls.map((c) => (
                <li key={c.id} className="flex items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{c.remote?.name || c.remote?.number}</p>
                    <p className="text-xs text-slate-500 dark:text-slate-400">
                      {c.direction === CallDirection.INCOMING ? "Incoming" : "Outgoing"}
                      {c.codec && <> · {label(codecNames, c.codec)}</>}
                      {c.durationSecs !== undefined ? ` · ${duration(Number(c.durationSecs) + elapsed)}` : ""}
                    </p>
                  </div>
                  <Badge tone="blue">{callStateLabel(c.state)}</Badge>
                </li>
              ))}
            </ul>
          )}
          <div className="mt-4 flex flex-wrap gap-2 text-xs">
            {calls.features?.dnd && <Badge tone="red">Do Not Disturb</Badge>}
            {calls.features?.forwardAlways && <Badge tone="amber">Forwarding to {calls.features.forwardAlways}</Badge>}
            {newMessages > 0 && <Badge tone="blue">{newMessages} {newMessages === 1 ? "voicemail" : "voicemails"}</Badge>}
            {calls.missedUnseen > 0 && <Badge tone="amber">{calls.missedUnseen} missed {calls.missedUnseen === 1 ? "call" : "calls"}</Badge>}
          </div>
        </Card>

        <Card title="Network" actions={<Badge tone={net.phase === NetPhase.UP ? "green" : net.phase === NetPhase.FAILED ? "red" : "amber"}>{label(netPhaseNames, net.phase, NetPhase[net.phase])}</Badge>}>
          <KeyValue
            rows={[
              ["Address", net.ipv4Address && <>{net.ipv4Address}/{net.ipv4PrefixLen}{net.ipv4Source && <> ({label(addressSourceNames, net.ipv4Source)})</>}</>],
              ["Gateway", net.ipv4Gateway],
              ["DNS", net.dns.join(", ")],
              ["LAN port", net.lan?.up ? `${net.lan.speed} Mbps, ${net.lan.fullDuplex ? "full" : "half"} duplex` : "No link"],
              ["PC port", !net.pcPortEnabled ? "Disabled" : net.pc?.up ? `${net.pc.speed} Mbps` : "No link"],
              ["Voice VLAN", net.voiceVlan !== undefined ? <>{net.voiceVlan}{net.vlanSource && <> ({label(vlanSourceNames, net.vlanSource)})</>}</> : "None"],
              ["Switch", net.lldpPresent ? `${net.lldpSystemName} ${net.lldpPortId}` : ""],
              ["MAC address", net.mac],
            ]}
          />
        </Card>

        <Card title="Provisioning" actions={<Badge tone={prov.phase === ProvisionPhase.PROVISIONED ? "green" : prov.phase === ProvisionPhase.FAILED ? "red" : "gray"}>{label(provisionPhaseNames, prov.phase, ProvisionPhase[prov.phase])}</Badge>}>
          <KeyValue
            rows={[
              ["Server", prov.server && <>{prov.server.name} ({label(discoverySourceNames, prov.server.source)})</>],
              ["Software", prov.serverUserAgent],
              ["User", prov.assignedUser],
              ["Session", prov.attached ? "Connected" : "Disconnected"],
              ["Last provisioned", dateTime(prov.lastSuccessUnix)],
              ["Error", prov.failureReason],
            ]}
          />
        </Card>

        <Card title="System">
          <KeyValue
            rows={[
              ["Hostname", sys.hostname],
              ["Uptime", duration(Number(sys.uptimeSecs) + elapsed)],
              ["Time", `${dateTime(Number(sys.unixTime) + elapsed)}${sys.timeSynced ? "" : " (not synced)"}`],
              ["Time zone", sys.timezone],
              ["Memory", `${Math.round(sys.memTotalKib / 1024)} MB`],
              ["Last reboot", sys.lastRebootReason],
              ["Firmware", status.firmware ? `${status.firmware.runningVersion} (bank ${status.firmware.runningBank})` : ""],
              ["User agent", sys.userAgent],
            ]}
          />
        </Card>

        {sys.hasExpansion && <ExpansionCard />}

        <StatusCard status={status} />
      </div>
    </>
  );
}
