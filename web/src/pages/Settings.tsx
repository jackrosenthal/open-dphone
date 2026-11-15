import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useMemo, useState } from "react";
import { Badge, Button, Card, Field, Input, PageHeader, Select, Spinner, Toggle, td, th } from "../components/ui";
import { ConfigService, SettingSource, type IdleSoftkeys as IdleSoftkeysMessage, type Setting } from "../gen/dphone/v1/config_pb";
import { useStatus } from "../lib/status";
import { ErrorNote } from "./common";

type Kind = { type: "bool" } | { type: "number"; min: number; max: number } | { type: "text" } | { type: "choice"; options: [string, string][] };
/// `grayOnly`: the setting exists only on the gray-display models.
type Item = { id: string; label: string; hint?: string; grayOnly?: boolean } & Kind;

// Settings with a proper form.  Everything else stays reachable in the
// table below by its stock setting ID.
const groups: { title: string; items: Item[] }[] = [
  {
    title: "Sound",
    items: [
      { id: "ringer_volume", label: "Ringer volume", type: "number", min: 0, max: 10 },
      { id: "handset_volume", label: "Handset volume", type: "number", min: 0, max: 10 },
      { id: "headset_volume", label: "Headset volume", type: "number", min: 0, max: 10 },
      { id: "speaker_volume", label: "Speaker volume", type: "number", min: 0, max: 10 },
      { id: "default_ringtone", label: "Ringtone", type: "text" },
      { id: "reset_call_volume", label: "Reset call volume after each call", type: "bool" },
      { id: "lock_ringing_volume", label: "Lock the ringer volume", type: "bool" },
      { id: "call_waiting_tone", label: "Call waiting tone", type: "bool" },
      { id: "ring_headset_only", label: "Ring in the headset only", type: "bool" },
      { id: "headset_answer", label: "Answer with the headset", type: "bool" },
      { id: "ehs", label: "Electronic hook switch", type: "choice", options: [["auto", "Automatic"], ["plantronics", "Plantronics"], ["jabra_iq", "Jabra"], ["none", "None"]] },
    ],
  },
  {
    title: "Display",
    items: [
      { id: "brightness", label: "Brightness", type: "number", min: 0, max: 10 },
      { id: "contrast", label: "Contrast", type: "number", min: 0, max: 10, grayOnly: true },
      { id: "backlight_timeout", label: "Backlight timeout (seconds)", type: "number", min: 0, max: 86400, hint: "0 keeps it on." },
      { id: "dim_backlight", label: "Dim the backlight when idle", type: "bool" },
      { id: "small_clock", label: "Small clock on the idle screen", type: "bool" },
      { id: "name_format", label: "Name format", type: "choice", options: [["first_last", "First Last"], ["last_first", "Last, First"]] },
    ],
  },
  {
    title: "Date and time",
    items: [
      { id: "time_zone", label: "Time zone", type: "text", hint: "IANA name, e.g., America/Chicago." },
      { id: "ntp_server", label: "NTP server", type: "text" },
      { id: "ntp_resync", label: "NTP interval (seconds)", type: "number", min: 60, max: 604800 },
      { id: "ntp_prefer_option42", label: "Prefer the DHCP-supplied NTP server", type: "bool" },
      { id: "time_short_format", label: "Time format", type: "text" },
      { id: "date_short_format", label: "Date format", type: "text" },
    ],
  },
  {
    title: "SIP",
    items: [
      { id: "transport_udp_enabled", label: "UDP", type: "bool" },
      { id: "transport_udp_port", label: "UDP port", type: "number", min: 1, max: 65535 },
      { id: "transport_tcp_enabled", label: "TCP", type: "bool" },
      { id: "transport_tcp_port", label: "TCP port", type: "number", min: 1, max: 65535 },
      { id: "transport_tls_allowed", label: "TLS", type: "bool" },
      { id: "transport_tls_port", label: "TLS port", type: "number", min: 1, max: 65535 },
      { id: "allow_insecure_ssl", label: "Accept unverified TLS certificates", type: "bool" },
      { id: "accept_local_calls", label: "Accept calls from", type: "choice", options: [["any", "Anyone"], ["host", "Registered servers only"], ["none", "Nobody"]] },
      { id: "enable_check_sync", label: "Honor check-sync NOTIFY", type: "bool" },
      { id: "default_dial_plan", label: "Dial plan", type: "text" },
    ],
  },
  {
    title: "Logging",
    items: [
      { id: "log_level", label: "Log level", type: "choice", options: [["error", "Error"], ["warning", "Warning"], ["information", "Information"], ["debug", "Debug"]] },
      { id: "enable_logging", label: "Send logs to a syslog server", type: "bool" },
      { id: "log_server", label: "Syslog server", type: "text" },
      { id: "log_port", label: "Syslog port", type: "number", min: 1, max: 65535 },
    ],
  },
  {
    title: "Access",
    items: [
      { id: "web_ui_enabled", label: "Web interface", type: "bool" },
    ],
  },
];

const truthy = (v: string) => v === "1" || v.toLowerCase() === "true" || v.toLowerCase() === "yes";

function Control({ item, setting, value, onChange }: { item: Item; setting?: Setting; value: string; onChange: (v: string) => void }) {
  const frozen = setting?.frozen;
  const label = (
    <>
      {item.label}
      {frozen && <span className="ml-2 text-xs font-normal text-slate-400">locked</span>}
    </>
  );
  if (item.type === "bool") {
    return (
      <div>
        <Toggle label={item.label + (frozen ? " (locked)" : "")} checked={truthy(value)} disabled={frozen} onChange={(v) => onChange(v ? "1" : "0")} />
        {item.hint && <p className="mt-1 pl-14 text-xs text-slate-500 dark:text-slate-400">{item.hint}</p>}
      </div>
    );
  }
  return (
    <label className="block">
      <span className="mb-1 block text-sm font-medium text-slate-700 dark:text-slate-300">{label}</span>
      {item.type === "choice" ? (
        <Select value={value} disabled={frozen} onChange={(e) => onChange(e.target.value)}>
          {!item.options.some(([v]) => v === value) && <option value={value}>{value || "(not set)"}</option>}
          {item.options.map(([v, l]) => (
            <option key={v} value={v}>
              {l}
            </option>
          ))}
        </Select>
      ) : item.type === "number" ? (
        <Input type="number" min={item.min} max={item.max} value={value} disabled={frozen} onChange={(e) => onChange(e.target.value)} />
      ) : (
        <Input value={value} disabled={frozen} onChange={(e) => onChange(e.target.value)} />
      )}
      {item.hint && <span className="mt-1 block text-xs text-slate-500 dark:text-slate-400">{item.hint}</span>}
    </label>
  );
}

/// The softkeys of the phone's idle screen, in order.  Each change is
/// saved as it is made.
function IdleSoftkeys() {
  const [error, setError] = useState<unknown>();
  const [saved, setSaved] = useState<IdleSoftkeysMessage>();
  const loaded = useQuery(ConfigService.method.getIdleSoftkeys, {});
  const set = useMutation(ConfigService.method.setIdleSoftkeys, {
    onError: setError,
    onSuccess: (r) => {
      setError(undefined);
      setSaved(r.idleSoftkeys);
    },
  });
  const keys = saved ?? loaded.data?.idleSoftkeys;
  const names = keys?.softkeys ?? [];
  const label = (name: string) => keys?.choices.find((c) => c.name === name)?.label ?? name;
  const fixed = !keys || keys.locked || set.isPending;
  const save = (softkeys: string[]) => set.mutate({ softkeys });
  const move = (i: number, by: number) => {
    const next = [...names];
    [next[i], next[i + by]] = [next[i + by], next[i]];
    save(next);
  };
  const origin = keys?.locked ? "Set by the provisioning server." : keys?.source === SettingSource.PROVISIONED ? "Provisioned." : "";
  return (
    <Card
      title="Idle screen softkeys"
      actions={
        keys?.source === SettingSource.LOCAL &&
        !keys.locked && (
          <Button size="sm" variant="ghost" disabled={set.isPending} onClick={() => save([])}>
            Revert
          </Button>
        )
      }
    >
      <div className="space-y-3">
        <ErrorNote error={error ?? loaded.error} />
        <ol className="divide-y divide-slate-100 dark:divide-slate-800">
          {names.map((name, i) => (
            <li key={name + i} className="flex items-center gap-1 py-1.5 text-sm">
              <span className="w-6 text-slate-400">{i + 1}</span>
              <span className="flex-1">{label(name)}</span>
              <Button size="sm" variant="ghost" aria-label="Move up" disabled={fixed || i === 0} onClick={() => move(i, -1)}>
                ↑
              </Button>
              <Button size="sm" variant="ghost" aria-label="Move down" disabled={fixed || i === names.length - 1} onClick={() => move(i, 1)}>
                ↓
              </Button>
              <Button size="sm" variant="ghost" aria-label="Remove" disabled={fixed || names.length === 1} onClick={() => save(names.filter((_, j) => j !== i))}>
                ✕
              </Button>
            </li>
          ))}
        </ol>
        <Select aria-label="Add a softkey" value="" disabled={fixed} onChange={(e) => e.target.value && save([...names, e.target.value])}>
          <option value="">Add…</option>
          {(keys?.choices ?? [])
            .filter((c) => !names.includes(c.name))
            .map((c) => (
              <option key={c.name} value={c.name}>
                {c.label}
              </option>
            ))}
        </Select>
        {origin && <p className="text-xs text-slate-500 dark:text-slate-400">{origin}</p>}
      </div>
    </Card>
  );
}

export function Settings() {
  const list = useQuery(ConfigService.method.listSettings, {});
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [filter, setFilter] = useState("");
  const [showAll, setShowAll] = useState(false);
  const [error, setError] = useState<unknown>();
  const skin = useStatus().status?.system?.skin;
  const done = () => {
    setDraft({});
    setError(undefined);
    list.refetch();
  };
  const save = useMutation(ConfigService.method.setSettings, { onSuccess: done, onError: setError });
  const unset = useMutation(ConfigService.method.unsetSettings, { onSuccess: done, onError: setError });

  const byId = useMemo(() => new Map((list.data?.settings ?? []).map((s) => [s.id, s])), [list.data]);
  if (list.error) return <ErrorNote error={list.error} />;
  if (!list.data) return <Spinner />;

  const value = (id: string) => draft[id] ?? byId.get(id)?.value ?? "";
  const change = (id: string, v: string) => {
    const next = { ...draft };
    if (v === (byId.get(id)?.value ?? "")) delete next[id];
    else next[id] = v;
    setDraft(next);
  };
  const changes = Object.entries(draft).map(([id, v]) => ({ id, value: v }));
  const q = filter.trim().toLowerCase();
  const rows = list.data.settings.filter((s) => !q || s.id.toLowerCase().includes(q) || s.value.toLowerCase().includes(q));

  return (
    <>
      <PageHeader
        title="Phone settings"
        actions={
          <>
            {changes.length > 0 && <Badge tone="amber">{changes.length} unsaved</Badge>}
            {changes.length > 0 && <Button onClick={() => setDraft({})}>Discard</Button>}
            <Button variant="primary" disabled={changes.length === 0} busy={save.isPending} onClick={() => save.mutate({ changes })}>
              Save
            </Button>
          </>
        }
      />
      <div className="space-y-5">
        <ErrorNote error={error} />
        <div className="grid gap-5 lg:grid-cols-2">
          {groups.map((g) => (
            <Card key={g.title} title={g.title}>
              <div className="grid gap-4 sm:grid-cols-2">
                {g.items.filter((item) => !item.grayOnly || skin !== "color").map((item) => (
                  <div key={item.id} className={item.type === "bool" || item.id === "default_dial_plan" ? "sm:col-span-2" : ""}>
                    <Control item={item} setting={byId.get(item.id)} value={value(item.id)} onChange={(v) => change(item.id, v)} />
                  </div>
                ))}
              </div>
            </Card>
          ))}
          <IdleSoftkeys />
        </div>

        <Card
          title={`All settings (${list.data.settings.length})`}
          actions={
            <Button size="sm" onClick={() => setShowAll(!showAll)}>
              {showAll ? "Hide" : "Show"}
            </Button>
          }
        >
          {showAll && (
            <>
              <Field label="Filter" className="mb-3 max-w-sm">
                <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Setting ID or value" />
              </Field>
              <div className="overflow-x-auto">
                <table className="w-full">
                  <thead>
                    <tr>
                      <th className={th}>Setting</th>
                      <th className={th}>Value</th>
                      <th className={th} />
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                    {rows.map((s) => (
                      <tr key={s.id}>
                        <td className={`${td} whitespace-nowrap font-mono text-xs`}>
                          {s.id} {s.local && <Badge tone="blue">local</Badge>} {s.frozen && <Badge>locked</Badge>}
                        </td>
                        <td className={`${td} w-full`}>
                          {s.secret ? (
                            <span className="text-xs text-slate-400">hidden</span>
                          ) : (
                            <Input className="!py-1 font-mono !text-xs" value={value(s.id)} disabled={s.frozen} onChange={(e) => change(s.id, e.target.value)} aria-label={s.id} />
                          )}
                        </td>
                        <td className={`${td} text-right`}>
                          {s.local && !s.secret && (
                            <Button size="sm" variant="ghost" onClick={() => unset.mutate({ ids: [s.id] })}>
                              Revert
                            </Button>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </Card>
      </div>
    </>
  );
}
