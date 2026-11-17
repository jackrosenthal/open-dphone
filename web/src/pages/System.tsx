import { useMutation } from "@connectrpc/connect-query";
import { useRef, useState, type FormEvent } from "react";
import { Alert, Button, Card, ConfirmButton, Field, Input, KeyValue, PageHeader } from "../components/ui";
import { AuthService } from "../gen/dphone/v1/auth_pb";
import { NetworkService } from "../gen/dphone/v1/network_pb";
import { SystemService } from "../gen/dphone/v1/system_pb";
import { dateTime } from "../lib/format";
import { modelName } from "../lib/labels";
import { getSession, setSession } from "../lib/session";
import { useStatus } from "../lib/status";
import { EhsPortCard, MetricsSwitch, SshCard, WebSwitch } from "./Ssh";
import { ErrorNote } from "./common";
import { FirmwareCard } from "./Firmware";
import { SettingsCard, type Item } from "./Settings";

/// The form that changes the web password.
function PasswordForm() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const [done, setDone] = useState(false);
  const change = useMutation(AuthService.method.changePassword, {
    onSuccess: () => {
      setCurrent("");
      setNext("");
      setAgain("");
      setDone(true);
      const s = getSession();
      if (s.status === "authenticated") setSession({ status: "authenticated", info: { ...s.info, defaultPassword: false } });
    },
  });
  const mismatch = again !== "" && next !== again;
  function submit(e: FormEvent) {
    e.preventDefault();
    setDone(false);
    if (!mismatch) change.mutate({ currentPassword: current, newPassword: next });
  }
  return (
    <form onSubmit={submit} className="space-y-4">
      <Field label="Current password">
        <Input type="password" autoComplete="current-password" required value={current} onChange={(e) => setCurrent(e.target.value)} />
      </Field>
      <Field label="New password">
        <Input type="password" autoComplete="new-password" required minLength={4} value={next} onChange={(e) => setNext(e.target.value)} />
      </Field>
      <Field label="Confirm new password" hint={mismatch ? <span className="text-red-600">Passwords do not match.</span> : undefined}>
        <Input type="password" autoComplete="new-password" required value={again} onChange={(e) => setAgain(e.target.value)} />
      </Field>
      <ErrorNote error={change.error} />
      {done && <Alert tone="green" title="Password changed." />}
      <Button type="submit" variant="primary" busy={change.isPending} disabled={mismatch}>
        Change Password
      </Button>
    </form>
  );
}

const time: Item[] = [
  { id: "time_zone", label: "Time zone", type: "text", placeholder: "America/Chicago" },
  { id: "ntp_server", label: "NTP server", type: "text" },
  { id: "ntp_resync", label: "NTP interval (seconds)", type: "number", min: 60, max: 604800 },
  { id: "ntp_prefer_option42", label: "Use NTP server from DHCP", type: "bool" },
];

const logging: Item[] = [
  { id: "log_level", label: "Log level", type: "choice", options: [["error", "Error"], ["warning", "Warning"], ["information", "Info"], ["debug", "Debug"]] },
  { id: "enable_logging", label: "Remote syslog", type: "bool" },
  { id: "log_server", label: "Syslog server", type: "text" },
  { id: "log_port", label: "Syslog port", type: "number", min: 1, max: 65535 },
];

function DateTime() {
  const { status, elapsed } = useStatus();
  const sys = status?.system;
  const [error, setError] = useState<unknown>();
  const opts = { onError: setError, onSuccess: () => setError(undefined) };
  const setTime = useMutation(SystemService.method.setTime, opts);
  const sync = useMutation(NetworkService.method.syncTime, opts);
  return (
    <SettingsCard title="Date and Time" items={time}>
      <ErrorNote error={error} />
      <KeyValue
        rows={[
          ["Time", sys ? dateTime(Number(sys.unixTime) + elapsed) : ""],
          ["NTP", sys ? (sys.timeSynced ? "Synced" : "Not synced") : ""],
        ]}
      />
      <div className="flex flex-wrap gap-2">
        <Button size="sm" onClick={() => sync.mutate({})} busy={sync.isPending}>
          Sync Now
        </Button>
        <Button size="sm" onClick={() => setTime.mutate({ unixSecs: BigInt(Math.floor(Date.now() / 1000)) })} busy={setTime.isPending}>
          Use Browser Time
        </Button>
      </div>
    </SettingsCard>
  );
}

function Backup() {
  const model = modelName(useStatus().status?.system?.model);
  const [error, setError] = useState<unknown>();
  const [restored, setRestored] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const opts = { onError: setError };
  const reset = useMutation(SystemService.method.factoryReset, opts);
  const backup = useMutation(SystemService.method.createBackup, opts);
  const restore = useMutation(SystemService.method.restoreBackup, opts);

  async function download() {
    setError(undefined);
    const r = await backup.mutateAsync({});
    const url = URL.createObjectURL(new Blob([r.archive as BlobPart], { type: "application/gzip" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = r.filename;
    a.click();
    URL.revokeObjectURL(url);
  }
  async function upload(f: File) {
    setError(undefined);
    setRestored(false);
    await restore.mutateAsync({ archive: new Uint8Array(await f.arrayBuffer()) });
    setRestored(true);
  }

  return (
    <Card title="Backup and Reset" className="lg:col-span-2">
      <div className="space-y-4">
        <ErrorNote error={error} />
        {restored && <Alert tone="green" title="Backup restored. Reboot to apply." />}
        <div className="flex flex-wrap gap-2">
          <Button onClick={download} busy={backup.isPending}>
            Download Backup
          </Button>
          <input ref={file} type="file" accept=".gz,.tgz,application/gzip" className="hidden" onChange={(e) => e.target.files?.[0] && upload(e.target.files[0]).finally(() => (e.target.value = ""))} />
          <Button onClick={() => file.current?.click()} busy={restore.isPending}>
            Restore Backup
          </Button>
          <div className="flex-1" />
          <ConfirmButton label="Factory Reset" title="Factory reset?" confirmLabel="Factory Reset" requireText={model || "RESET"} onConfirm={() => reset.mutateAsync({ confirm: model })} />
        </div>
      </div>
    </Card>
  );
}

export function System() {
  return (
    <>
      <PageHeader title="System" />
      <div className="grid items-start gap-5 lg:grid-cols-2">
        <div className="space-y-5">
          <FirmwareCard />
          {/* The default-password notice links here. */}
          <div id="web-ui" className="scroll-mt-20">
            <Card title="Web UI">
              <div className="space-y-4">
                <WebSwitch />
                <div className="border-t border-slate-100 pt-4 dark:border-slate-800">
                  <PasswordForm />
                </div>
              </div>
            </Card>
          </div>
          <EhsPortCard />
        </div>
        <div className="space-y-5">
          <DateTime />
          <SettingsCard title="Logging" items={logging} after={<MetricsSwitch />} />
        </div>
        <SshCard />
        <Backup />
      </div>
    </>
  );
}
