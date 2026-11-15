import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useRef, useState, type FormEvent } from "react";
import { Badge, Button, Card, ConfirmButton, Empty, Field, Modal, Select, Toggle, td, th } from "../components/ui";
import { ConfigService } from "../gen/dphone/v1/config_pb";
import { SshService, type SshStatus } from "../gen/dphone/v1/ssh_pb";
import { SystemService } from "../gen/dphone/v1/system_pb";
import { ErrorNote } from "./common";
import { settingOrigin } from "./Settings";

/// The switch for the unauthenticated `/metrics` endpoint.
function MetricsSwitch({ onError }: { onError: (e: unknown) => void }) {
  const loaded = useQuery(SystemService.method.getMetricsEnabled, {});
  const set = useMutation(SystemService.method.setMetricsEnabled, { onError, onSuccess: () => (onError(undefined), loaded.refetch()) });
  const setting = loaded.data?.metricsEnabled;
  const origin = settingOrigin(setting);
  return (
    <>
      <Toggle label="Prometheus metrics" checked={setting?.value ?? false} disabled={!setting || setting.locked || set.isPending} onChange={(v) => set.mutate({ metricsEnabled: v })} />
      {origin && <span className="text-xs text-slate-500 dark:text-slate-400">{origin}</span>}
    </>
  );
}

/// The switch for this interface, which asks before it locks the
/// administrator out.
function WebSwitch({ onError }: { onError: (e: unknown) => void }) {
  const [asking, setAsking] = useState(false);
  const list = useQuery(ConfigService.method.listSettings, {});
  const set = useMutation(ConfigService.method.setSettings, { onError, onSuccess: () => (onError(undefined), setAsking(false), list.refetch()) });
  const setting = list.data?.settings.find((s) => s.id === "web_ui_enabled");
  const on = setting ? setting.value === "1" || setting.value.toLowerCase() === "true" || setting.value.toLowerCase() === "yes" : true;
  const save = (v: boolean) => set.mutate({ changes: [{ id: "web_ui_enabled", value: v ? "1" : "0" }] });
  return (
    <>
      <Toggle label="Web interface" checked={on} disabled={!setting || setting.frozen || set.isPending} onChange={(v) => (v ? save(true) : setAsking(true))} />
      {asking && (
        <Modal
          title="Disable the web interface?"
          onClose={() => setAsking(false)}
          footer={
            <>
              <Button onClick={() => setAsking(false)}>Cancel</Button>
              <Button variant="danger" busy={set.isPending} onClick={() => save(false)}>
                Disable
              </Button>
            </>
          }
        >
          {null}
        </Modal>
      )}
    </>
  );
}

/// The EHS header as seen when attaching a USB serial cable, with the
/// cable's wire colors and each pin's function from the phone's side.
function EhsPinout() {
  const x = (pin: number) => 35 + (pin - 1) * 50;
  const ink = "fill-slate-700 dark:fill-slate-200";
  const muted = "fill-slate-500 dark:fill-slate-400";
  // Each wire is drawn over a wider gray line, so that white shows on the
  // light theme and black on the dark one.
  const wire = (px: number, y1: number, color: string) => (
    <>
      <path d={`M${px} ${y1}V122`} className="stroke-slate-500" strokeWidth={8} strokeLinecap="round" />
      <path d={`M${px} ${y1}V122`} stroke={color} strokeWidth={5} strokeLinecap="round" />
    </>
  );
  const caption = (px: number, color: string, what: string) => (
    <>
      <text x={px} y={139} className={ink} fontWeight={600}>
        {color}
      </text>
      <text x={px} y={153} className={muted}>
        {what}
      </text>
    </>
  );
  return (
    <svg
      viewBox="0 0 320 160"
      className="block h-auto w-full max-w-sm"
      fontSize={11}
      textAnchor="middle"
      role="img"
      aria-label="Pin 1: ground, black wire. Pin 2: TX, white wire. Pin 4: 3.3 V, not connected. Pin 5: RX, green wire. Pin 6: ground. Red wire: not connected."
    >
      <rect x={8} y={26} width={304} height={36} rx={4} className="fill-slate-200 stroke-slate-400 dark:fill-slate-800 dark:stroke-slate-600" />
      {[1, 2, 3, 4, 5, 6].map((pin) => (
        <g key={pin}>
          <text x={x(pin)} y={18} className={pin === 3 ? muted : ink} fontWeight={pin === 3 ? 400 : 600}>
            {pin === 3 ? "Key" : pin}
          </text>
          {pin !== 3 && <rect x={x(pin) - 6} y={38} width={12} height={12} className="fill-amber-500" />}
        </g>
      ))}
      {wire(x(1), 44, "#111111")}
      {wire(x(2), 44, "#ffffff")}
      {wire(x(5), 44, "#16a34a")}
      {wire(x(6), 104, "#dc2626")}
      <text x={x(4)} y={78} className={ink}>
        3.3 V
      </text>
      <text x={x(4)} y={90} fontSize={10} className={muted}>
        Not connected
      </text>
      <text x={x(6)} y={78} className={ink}>
        GND
      </text>
      {caption(x(1), "Black", "GND")}
      {caption(x(2), "White", "TX")}
      {caption(x(5), "Green", "RX")}
      <text x={x(6)} y={139} className={ink} fontWeight={600}>
        Red
      </text>
      <text x={x(6)} y={152} fontSize={10} className={muted}>
        Not connected
      </text>
    </svg>
  );
}

/// UART A is the EHS headset adapter's unless the serial console is chosen.
function SerialPort({ onError }: { onError: (e: unknown) => void }) {
  const loaded = useQuery(SystemService.method.getSerialConsole, {});
  const set = useMutation(SystemService.method.setSerialConsole, { onError, onSuccess: () => (onError(undefined), loaded.refetch()) });
  const setting = loaded.data?.serialConsole;
  return (
    <div className="flex flex-wrap items-start gap-x-8 gap-y-4">
      <Field label="Serial port" hint={settingOrigin(setting) || undefined} className="w-full max-w-xs">
        <Select value={setting?.value ? "console" : "ehs"} disabled={!setting || setting.locked || set.isPending} onChange={(e) => set.mutate({ serialConsole: e.target.value === "console" })}>
          <option value="ehs">EHS headset</option>
          <option value="console">Console (115200 8N1)</option>
        </Select>
      </Field>
      <EhsPinout />
    </div>
  );
}

export function AccessCard() {
  const [status, setStatus] = useState<SshStatus>();
  const [error, setError] = useState<unknown>();
  const [key, setKey] = useState("");
  const file = useRef<HTMLInputElement>(null);
  const loaded = useQuery(SshService.method.getSsh, {});
  const done = (r: { status?: SshStatus }) => {
    setError(undefined);
    setStatus(r.status);
  };
  const opts = { onSuccess: done, onError: setError };
  const enable = useMutation(SshService.method.setSshEnabled, opts);
  const add = useMutation(SshService.method.addSshKey, { onSuccess: (r: { status?: SshStatus }) => (done(r), setKey("")), onError: setError });
  const remove = useMutation(SshService.method.removeSshKey, opts);

  const ssh = status ?? loaded.data?.status;
  const enabled = ssh?.enabled;
  const origin = settingOrigin(enabled);
  const keys = ssh?.keys ?? [];
  // The listener's real state, when it disagrees with the switch.
  const stopped = enabled?.value && ssh?.listening === false;

  function submit(e: FormEvent) {
    e.preventDefault();
    // A private key never leaves the browser (the phone refuses one too).
    if (/PRIVATE KEY|PuTTY-User-Key-File/.test(key)) {
      setKey("");
      setError(new Error("This is a private key. Use the public key (.pub) instead."));
    } else if (key.trim()) add.mutate({ key });
  }

  return (
    <Card title="Access" className="lg:col-span-2">
      <div className="space-y-4">
        <ErrorNote error={error ?? loaded.error} />
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
          <WebSwitch onError={setError} />
          <Toggle label="SSH" checked={enabled?.value ?? false} disabled={!ssh || enabled?.locked || enable.isPending} onChange={(v) => enable.mutate({ enabled: v })} />
          {origin && <span className="text-xs text-slate-500 dark:text-slate-400">{origin}</span>}
          {stopped && <Badge tone="red">SSH not running</Badge>}
          <MetricsSwitch onError={setError} />
        </div>
        <SerialPort onError={setError} />

        <div className="border-t border-slate-100 pt-4 dark:border-slate-800">
          <h3 className="mb-2 text-sm font-semibold">SSH Keys</h3>
          {keys.length === 0 ? (
            <Empty>No keys</Empty>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr>
                    <th className={th}>Type</th>
                    <th className={th}>Fingerprint</th>
                    <th className={th}>Comment</th>
                    <th className={th} />
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                  {keys.map((k) => (
                    <tr key={k.fingerprint}>
                      <td className={td}>
                        <span className="whitespace-nowrap">
                          {k.type}
                          {k.bits > 0 && <span className="text-slate-500"> {k.bits}</span>}
                        </span>
                      </td>
                      <td className={`${td} break-all font-mono text-xs`}>{k.fingerprint}</td>
                      <td className={`${td} break-all`}>{k.comment}</td>
                      <td className={`${td} text-right`}>
                        {k.provisioned ? (
                          <Badge>Provisioned</Badge>
                        ) : (
                          <ConfirmButton size="sm" variant="secondary" label="Remove" title={`Remove ${k.comment || k.type}?`} confirmLabel="Remove" onConfirm={() => remove.mutateAsync({ fingerprint: k.fingerprint })} />
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>

        <form onSubmit={submit} className="space-y-2">
          <textarea
            aria-label="Public key"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            rows={3}
            spellCheck={false}
            placeholder="ssh-ed25519 AAAA… name"
            className="block w-full rounded-lg border border-slate-300 bg-white px-3 py-2 font-mono text-xs text-slate-900 placeholder:text-slate-400 focus:border-brand-500 focus:outline-none focus:ring-2 focus:ring-brand-500/30 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-100"
          />
          <div className="flex gap-2">
            <Button type="submit" variant="primary" busy={add.isPending} disabled={!key.trim()}>
              Add Key
            </Button>
            <input
              ref={file}
              type="file"
              accept=".pub,text/plain"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                e.target.value = "";
                // A public key file is a few hundred bytes; anything large is not one.
                if (f && f.size <= 16384) f.text().then(setKey);
                else if (f) setError(new Error("File is too large to be a public key."));
              }}
            />
            <Button onClick={() => file.current?.click()}>Choose File</Button>
          </div>
        </form>
      </div>
    </Card>
  );
}
