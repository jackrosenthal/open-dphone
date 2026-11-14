import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useRef, useState, type FormEvent } from "react";
import { Badge, Button, Card, ConfirmButton, Empty, Toggle, td, th } from "../components/ui";
import { SettingSource, type BoolSetting } from "../gen/dphone/v1/config_pb";
import { SshService, type SshStatus } from "../gen/dphone/v1/ssh_pb";
import { ErrorNote } from "./common";

/// What to say beside one of Open DPhone's own settings about where its
/// value comes from; nothing when it was set here or is the default.
export function settingOrigin(s: BoolSetting | undefined): string {
  if (s?.locked) return "Set by the provisioning server.";
  if (s?.source === SettingSource.PROVISIONED) return "Provisioned.";
  return "";
}

export function SshCard() {
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
      setError(new Error("That is a private key, and it was not sent. Use the public key (the .pub file)."));
    } else if (key.trim()) add.mutate({ key });
  }

  return (
    <Card title="SSH" className="lg:col-span-2">
      <div className="space-y-4">
        <ErrorNote error={error ?? loaded.error} />
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
          <Toggle label="SSH server" checked={enabled?.value ?? false} disabled={!ssh || enabled?.locked || enable.isPending} onChange={(v) => enable.mutate({ enabled: v })} />
          {origin && <span className="text-xs text-slate-500 dark:text-slate-400">{origin}</span>}
          {stopped && <Badge tone="red">Not listening</Badge>}
        </div>

        <div className="border-t border-slate-100 pt-4 dark:border-slate-800">
          <h3 className="mb-2 text-sm font-semibold">Authorized keys</h3>
          {keys.length === 0 ? (
            <Empty>None</Empty>
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
              Add key
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
                else if (f) setError(new Error("That file is too large to be a public key."));
              }}
            />
            <Button onClick={() => file.current?.click()}>Choose .pub file</Button>
          </div>
        </form>
      </div>
    </Card>
  );
}
