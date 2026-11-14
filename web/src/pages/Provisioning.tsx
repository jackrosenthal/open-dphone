import { useMutation } from "@connectrpc/connect-query";
import { useState } from "react";
import { Alert, Badge, Button, Card, ConfirmButton, Empty, Field, Input, KeyValue, PageHeader, Spinner } from "../components/ui";
import { ProvisionPhase, ProvisioningService } from "../gen/dphone/v1/provisioning_pb";
import { dateTime, enumLabel } from "../lib/format";
import { useStatus } from "../lib/status";
import { ErrorNote } from "./common";

export function Provisioning() {
  const { status } = useStatus();
  const [server, setServer] = useState("");
  const [pin, setPin] = useState("");
  const [error, setError] = useState<unknown>();
  const opts = { onError: setError, onSuccess: () => setError(undefined) };
  const start = useMutation(ProvisioningService.method.startProvisioning, opts);
  const select = useMutation(ProvisioningService.method.selectUser, opts);
  const again = useMutation(ProvisioningService.method.reprovision, opts);
  const forget = useMutation(ProvisioningService.method.forgetProvisioning, opts);
  const skip = useMutation(ProvisioningService.method.skipProvisioning, opts);
  const p = status?.provisioning;
  if (!p) return <Spinner />;
  const tone = p.phase === ProvisionPhase.PROVISIONED ? "green" : p.phase === ProvisionPhase.FAILED ? "red" : p.phase === ProvisionPhase.STANDALONE || p.phase === ProvisionPhase.IDLE ? "gray" : "blue";

  return (
    <>
      <PageHeader
        title="Provisioning"
        actions={
          <>
            <Button onClick={() => again.mutate({})} busy={again.isPending} disabled={!p.server}>
              Fetch configuration again
            </Button>
            <ConfirmButton variant="secondary" label="Forget server" title="Forget the provisioning server?" message="The saved server is dropped and discovery starts over. Accounts from the server stop applying." confirmLabel="Forget" onConfirm={() => forget.mutateAsync({})} />
          </>
        }
      />
      <div className="space-y-5">
        <ErrorNote error={error} />
        {p.phase === ProvisionPhase.FAILED && (
          <Alert tone="red" title="Provisioning failed.">
            {p.failureReason}
            {p.failureCode !== undefined ? ` (code ${p.failureCode})` : ""}
          </Alert>
        )}
        {p.offeredFirmware && <Alert tone="blue" title={`The server offers firmware ${p.offeredFirmware}.`}>Server-pushed firmware is ignored unless enabled in the phone settings.</Alert>}
        <div className="grid gap-5 lg:grid-cols-2">
          <Card title="Status" actions={<Badge tone={tone}>{enumLabel(ProvisionPhase[p.phase], "")}</Badge>}>
            <KeyValue
              rows={[
                ["Server", p.server ? `${p.server.name}` : ""],
                ["Address", p.server?.address],
                ["Found by", p.server?.source],
                ["Server software", p.serverUserAgent],
                ["Assigned to", p.assignedUser],
                ["Session", p.attached ? "Attached (pushes can arrive)" : "Not attached"],
                ["Configuration MD5", p.configMd5 && <code className="text-xs">{p.configMd5}</code>],
                ["Last success", dateTime(p.lastSuccessUnix)],
              ]}
            />
          </Card>

          <Card title="Server">
            <form
              className="flex items-end gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                start.mutate({ server: server.trim() });
              }}
            >
              <Field label="Server address" className="flex-1">
                <Input value={server} onChange={(e) => setServer(e.target.value)} placeholder="host:port" />
              </Field>
              <Button type="submit" variant="primary" busy={start.isPending} >
                {server.trim() ? "Use" : "Discover"}
              </Button>
            </form>
            <p className="mb-2 mt-4 text-xs font-semibold uppercase tracking-wide text-slate-500">Discovered</p>
            {p.discovered.length === 0 ? (
              <Empty>None</Empty>
            ) : (
              <ul className="divide-y divide-slate-100 dark:divide-slate-800">
                {p.discovered.map((d) => (
                  <li key={d.address} className="flex items-center justify-between gap-3 py-2">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{d.name}</p>
                      <p className="truncate text-xs text-slate-500">
                        {d.address} · {d.source}
                      </p>
                    </div>
                    <Button size="sm" onClick={() => start.mutate({ server: d.address })}>
                      Use
                    </Button>
                  </li>
                ))}
              </ul>
            )}
            <div className="mt-4 border-t border-slate-100 pt-4 dark:border-slate-800">
              <Button onClick={() => skip.mutate({})}>Run without a provisioning server</Button>
            </div>
          </Card>
        </div>

        {p.phase === ProvisionPhase.AWAITING_USER && (
          <Card title="Choose who uses this phone">
            <div className="mb-4 max-w-xs">
              <Field label="PIN (if the server asks for one)">
                <Input type="password" value={pin} onChange={(e) => setPin(e.target.value)} autoComplete="off" />
              </Field>
            </div>
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {p.users.map((u) => (
                <li key={u.id} className="flex items-center justify-between gap-3 py-2">
                  <span className="text-sm">
                    <span className="font-medium">{u.name || u.id}</span> <span className="text-slate-400">{u.id}</span> {u.needsPin && <Badge>PIN</Badge>}
                  </span>
                  <Button size="sm" variant="primary" onClick={() => select.mutate({ id: u.id, pin })}>
                    Select
                  </Button>
                </li>
              ))}
            </ul>
          </Card>
        )}
      </div>
    </>
  );
}
