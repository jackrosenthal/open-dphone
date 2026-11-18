import { create } from "@bufbuild/protobuf";
import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useState } from "react";
import { Alert, Badge, Button, Card, ConfirmButton, Dpma, Empty, Field, Input, Modal, PageHeader, Select, Spinner, Toggle } from "../components/ui";
import { AccountConfigSchema, AccountsService, SipTransport, type AccountConfig } from "../gen/opendeskphone/v1/accounts_pb";
import { useStatus } from "../lib/status";
import { ErrorNote, RegistrationBadge } from "./common";
import { SettingsCard, type Item } from "./Settings";

// SIP settings shared by every account.
const sip: Item[] = [
  { id: "transport_udp_port", label: "UDP port", type: "number", min: 1, max: 65535 },
  { id: "transport_tcp_port", label: "TCP port", type: "number", min: 1, max: 65535 },
  { id: "transport_tls_port", label: "TLS port", type: "number", min: 1, max: 65535 },
  { id: "accept_local_calls", label: "Accept calls from", type: "choice", options: [["any", "Anyone"], ["host", "Registered servers"], ["none", "No one"]] },
  { id: "default_dial_plan", label: "Dial plan", type: "text", wide: true },
  { id: "transport_udp_enabled", label: "UDP", type: "bool" },
  { id: "transport_tcp_enabled", label: "TCP", type: "bool" },
  { id: "transport_tls_allowed", label: "TLS", type: "bool" },
  { id: "allow_insecure_ssl", label: "Allow untrusted TLS certificates", type: "bool" },
  { id: "enable_check_sync", label: "Allow check-sync", type: "bool" },
];

function Editor({ account, onSave, onClose, busy, error }: { account: AccountConfig; onSave: (a: AccountConfig) => void; onClose: () => void; busy: boolean; error: unknown }) {
  const [a, setA] = useState(account);
  const set = <K extends keyof AccountConfig>(k: K, v: AccountConfig[K]) => setA({ ...a, [k]: v });
  const text = (k: "username" | "authname" | "password" | "displayName" | "lineLabel" | "server" | "alternateServer" | "outboundProxy" | "voicemail", label: string, extra?: { type?: string; hint?: string; placeholder?: string }) => (
    <Field label={label} hint={extra?.hint}>
      <Input type={extra?.type ?? "text"} value={a[k]} placeholder={extra?.placeholder} autoComplete="off" onChange={(e) => set(k, e.target.value)} />
    </Field>
  );
  const transport = (k: "transport" | "alternateTransport") => (
    <Field label="Transport">
      <Select value={a[k] || SipTransport.UDP} onChange={(e) => set(k, Number(e.target.value))}>
        <option value={SipTransport.UDP}>UDP</option>
        <option value={SipTransport.TCP}>TCP</option>
        <option value={SipTransport.TLS}>TLS</option>
      </Select>
    </Field>
  );
  const port = (k: "port" | "alternatePort" | "outboundPort", label: string) => (
    <Field label={label}>
      <Input type="number" min={0} max={65535} value={a[k] || ""} placeholder="Default" onChange={(e) => set(k, Number(e.target.value) || 0)} />
    </Field>
  );
  return (
    <Modal
      title={`Line ${a.index + 1}`}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" busy={busy} onClick={() => onSave(a)}>
            Save
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <div className="flex flex-wrap gap-6">
          <Toggle label="Enabled" checked={a.enabled} onChange={(v) => set("enabled", v)} />
          <Toggle label="Register" checked={a.register} onChange={(v) => set("register", v)} />
          <Toggle label="SRTP" checked={a.srtp} onChange={(v) => set("srtp", v)} />
        </div>
        <div className="grid gap-4 sm:grid-cols-2">
          {text("username", "Username")}
          {text("authname", "Auth username", { placeholder: "Same as username" })}
          {text("password", "Password", { type: "password", placeholder: a.hasPassword ? "Unchanged" : "", hint: undefined })}
          {text("displayName", "Display name")}
          {text("lineLabel", "Line label")}
          {text("voicemail", "Voicemail number")}
        </div>
        <fieldset className="grid gap-4 sm:grid-cols-[1fr_7rem_7rem]">
          <legend className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-500">Server</legend>
          {text("server", "Host")}
          {port("port", "Port")}
          {transport("transport")}
        </fieldset>
        <fieldset className="grid gap-4 sm:grid-cols-[1fr_7rem_7rem]">
          <legend className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-500">Backup Server</legend>
          {text("alternateServer", "Host", { placeholder: "None" })}
          {port("alternatePort", "Port")}
          {transport("alternateTransport")}
        </fieldset>
        <div className="grid gap-4 sm:grid-cols-[1fr_7rem]">
          {text("outboundProxy", "Outbound proxy", { placeholder: "None" })}
          {port("outboundPort", "Port")}
        </div>
        <ErrorNote error={error} />
      </div>
    </Modal>
  );
}

export function Accounts() {
  const { status } = useStatus();
  const list = useQuery(AccountsService.method.listAccounts, {});
  const [editing, setEditing] = useState<AccountConfig>();
  const [error, setError] = useState<unknown>();
  const done = () => {
    setEditing(undefined);
    setError(undefined);
    list.refetch();
  };
  const save = useMutation(AccountsService.method.setAccounts, { onSuccess: done, onError: setError });
  const clear = useMutation(AccountsService.method.clearLocalAccounts, { onSuccess: done, onError: setError });
  const rereg = useMutation(AccountsService.method.reregister, { onError: setError });

  if (!list.data) {
    return list.error ? <ErrorNote error={list.error} /> : <Spinner />;
  }
  const { accounts, maxAccounts, local } = list.data;
  const live = status?.accounts ?? list.data.status;
  // Accounts that are not the phone's own are the provisioning server's.
  const dpma = !local && accounts.length > 0;
  const free = Array.from({ length: maxAccounts }, (_, i) => i).filter((i) => !accounts.some((a) => a.index === i));

  const saveOne = (a: AccountConfig) => save.mutate({ accounts: [...accounts.filter((x) => x.index !== a.index), a] });
  const remove = (index: number) => save.mutateAsync({ accounts: accounts.filter((x) => x.index !== index) });

  return (
    <>
      <PageHeader
        title="Accounts"
        actions={
          <>
            <Button onClick={() => rereg.mutate({})}>Re-register</Button>
            <Dpma on={dpma}>
              <Button
                variant="primary"
                disabled={dpma || free.length === 0}
                onClick={() => setEditing(create(AccountConfigSchema, { index: free[0], enabled: true, register: true, transport: SipTransport.UDP }))}
              >
                Add Account
              </Button>
            </Dpma>
          </>
        }
      />
      <div className="space-y-5">
        {!editing && <ErrorNote error={error} />}
        {local && (
          <Alert
            tone="blue"
            title="Local accounts override provisioned accounts."
            action={<ConfirmButton size="sm" variant="secondary" label="Revert" title="Revert to provisioned accounts?" confirmLabel="Revert" onConfirm={() => clear.mutateAsync({})} />}
          />
        )}
        <Card>
          {accounts.length === 0 ? (
            <Empty>No accounts</Empty>
          ) : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {accounts.map((a) => {
                const st = live.find((s) => s.index === a.index);
                return (
                  <li key={a.index} className="flex flex-wrap items-center justify-between gap-3 py-3 first:pt-0 last:pb-0">
                    <div className="min-w-0">
                      <p className="font-medium">
                        <span className="mr-2 text-slate-400">Line {a.index + 1}</span>
                        {a.lineLabel || a.username}
                        {!a.enabled && <span className="ml-2"><Badge>Disabled</Badge></span>}
                      </p>
                      <p className="truncate text-sm text-slate-500 dark:text-slate-400">
                        {a.username}@{a.server}
                        {a.port ? `:${a.port}` : ""} · {SipTransport[a.transport] ?? "UDP"}
                        {a.srtp ? " · SRTP" : ""}
                        {a.alternateServer ? ` · backup ${a.alternateServer}` : ""}
                        {st?.reason ? ` · ${st.reason}` : ""}
                      </p>
                    </div>
                    <div className="flex items-center gap-2">
                      {st && <RegistrationBadge a={st} />}
                      {dpma ? (
                        <Dpma on className="flex items-center gap-2">
                          <Button size="sm" disabled>
                            Edit
                          </Button>
                          <Button size="sm" disabled>
                            Remove
                          </Button>
                        </Dpma>
                      ) : (
                        <>
                          <Button size="sm" onClick={() => setEditing({ ...a, password: "" })}>
                            Edit
                          </Button>
                          <ConfirmButton size="sm" variant="secondary" label="Remove" title={`Remove Line ${a.index + 1}?`} confirmLabel="Remove" onConfirm={() => remove(a.index)} />
                        </>
                      )}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </Card>
        <SettingsCard title="SIP" items={sip} />
      </div>
      {editing && <Editor account={editing} busy={save.isPending} error={error} onSave={saveOne} onClose={() => { setEditing(undefined); setError(undefined); }} />}
    </>
  );
}
