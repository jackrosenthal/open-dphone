import { useMutation } from "@connectrpc/connect-query";
import { useRef, useState, type FormEvent } from "react";
import { Alert, Button, Card, ConfirmButton, Field, Input, KeyValue, PageHeader } from "../components/ui";
import { AuthService } from "../gen/dphone/v1/auth_pb";
import { SystemService } from "../gen/dphone/v1/system_pb";
import { dateTime, duration } from "../lib/format";
import { getSession, setSession } from "../lib/session";
import { useStatus } from "../lib/status";
import { ErrorNote } from "./common";

function Password() {
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
    <Card title="Administrator password">
      <form onSubmit={submit} className="space-y-4">
        <Field label="Current password">
          <Input type="password" autoComplete="current-password" required value={current} onChange={(e) => setCurrent(e.target.value)} />
        </Field>
        <Field label="New password">
          <Input type="password" autoComplete="new-password" required minLength={4} value={next} onChange={(e) => setNext(e.target.value)} />
        </Field>
        <Field label="New password again" hint={mismatch ? <span className="text-red-600">The passwords differ.</span> : undefined}>
          <Input type="password" autoComplete="new-password" required value={again} onChange={(e) => setAgain(e.target.value)} />
        </Field>
        <ErrorNote error={change.error} />
        {done && <Alert tone="green" title="Password changed. Other sessions were logged out." />}
        <Button type="submit" variant="primary" busy={change.isPending} disabled={mismatch}>
          Change password
        </Button>
        <p className="text-xs text-slate-500 dark:text-slate-400">The password is stored in the stock format, so it keeps working if the phone is returned to stock firmware.</p>
      </form>
    </Card>
  );
}

export function System() {
  const { status, elapsed } = useStatus();
  const sys = status?.system;
  const [error, setError] = useState<unknown>();
  const [note, setNote] = useState("");
  const file = useRef<HTMLInputElement>(null);
  const opts = { onError: setError };
  const reboot = useMutation(SystemService.method.reboot, opts);
  const reset = useMutation(SystemService.method.factoryReset, opts);
  const setTime = useMutation(SystemService.method.setTime, opts);
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
    setNote("");
    const archive = new Uint8Array(await f.arrayBuffer());
    const r = await restore.mutateAsync({ archive });
    setNote(`Restored ${r.files} ${r.files === 1 ? "file" : "files"}. Restart the phone to apply everything.`);
  }

  return (
    <>
      <PageHeader title="System" />
      <div className="space-y-5">
        <ErrorNote error={error} />
        {note && <Alert tone="green" title={note} />}
        <div className="grid gap-5 lg:grid-cols-2">
          <Card title="Restart and reset">
            <div className="space-y-4">
              <div className="flex items-center justify-between gap-4">
                <p className="text-sm text-slate-600 dark:text-slate-300">Restart the phone. Calls in progress end.</p>
                <ConfirmButton variant="secondary" label="Restart" title="Restart the phone?" message="Calls in progress end, and this page reconnects when the phone is back." confirmLabel="Restart" onConfirm={() => reboot.mutateAsync({}).then(() => setNote("The phone is restarting."))} />
              </div>
              <div className="flex items-center justify-between gap-4 border-t border-slate-100 pt-4 dark:border-slate-800">
                <p className="text-sm text-slate-600 dark:text-slate-300">Erase all settings, accounts, contacts, and the administrator password, then restart.</p>
                <ConfirmButton label="Factory reset" title="Erase everything?" message="All settings stored on the phone are deleted, as the stock factory reset does. The network identity (MAC address) is kept." confirmLabel="Erase and restart" requireText={sys?.model ?? "RESET"} onConfirm={() => reset.mutateAsync({ confirm: sys?.model ?? "" }).then(() => setNote("The phone is resetting."))} />
              </div>
            </div>
          </Card>

          <Card title="Backup and restore">
            <div className="space-y-4">
              <p className="text-sm text-slate-600 dark:text-slate-300">A backup holds the settings changed on the phone, the stored accounts, and the contact lists. It does not hold the phone's identity keys or provisioned configuration.</p>
              <div className="flex gap-2">
                <Button onClick={download} busy={backup.isPending}>
                  Download backup
                </Button>
                <input ref={file} type="file" accept=".gz,.tgz,application/gzip" className="hidden" onChange={(e) => e.target.files?.[0] && upload(e.target.files[0]).finally(() => (e.target.value = ""))} />
                <Button onClick={() => file.current?.click()} busy={restore.isPending}>
                  Restore from backup
                </Button>
              </div>
            </div>
          </Card>

          <Password />

          <Card title="Clock">
            <KeyValue
              rows={[
                ["Phone time", sys ? dateTime(Number(sys.unixTime) + elapsed) : ""],
                ["Synchronized", sys?.timeSynced ? "Yes (NTP)" : "No"],
                ["Time zone", sys?.timezone],
                ["Uptime", sys ? duration(Number(sys.uptimeSecs) + elapsed) : ""],
              ]}
            />
            <div className="mt-4">
              <Button onClick={() => setTime.mutate({ unixSecs: BigInt(Math.floor(Date.now() / 1000)) })} busy={setTime.isPending}>
                Set from this computer
              </Button>
            </div>
          </Card>
        </div>
      </div>
    </>
  );
}
