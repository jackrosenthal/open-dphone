import { useMutation } from "@connectrpc/connect-query";
import { useState } from "react";
import { Alert, Badge, Button, Card, ConfirmButton, Empty, Field, Input, KeyValue, PageHeader, Spinner } from "../components/ui";
import { DiscoverySource, ProvisionPhase, ProvisioningService } from "../gen/dphone/v1/provisioning_pb";
import { dateTime } from "../lib/format";
import { discoverySourceNames, label, provisionPhaseNames } from "../lib/labels";
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
              Reprovision
            </Button>
            <ConfirmButton variant="secondary" label="Forget Server" title="Forget the provisioning server?" confirmLabel="Forget Server" onConfirm={() => forget.mutateAsync({})} />
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
        {p.offeredFirmware && <Alert tone="blue" title={`Firmware ${p.offeredFirmware} is available from the server.`} />}
        <div className="grid gap-5 lg:grid-cols-2">
          <Card title="Status" actions={<Badge tone={tone}>{label(provisionPhaseNames, p.phase, ProvisionPhase[p.phase])}</Badge>}>
            <KeyValue
              rows={[
                ["Server", p.server ? `${p.server.name}` : ""],
                ["Address", p.server?.address],
                ["Discovery", p.server && p.server.source !== DiscoverySource.UNSPECIFIED ? label(discoverySourceNames, p.server.source, DiscoverySource[p.server.source]) : ""],
                ["Software", p.serverUserAgent],
                ["User", p.assignedUser],
                ["Session", p.attached ? "Connected" : "Disconnected"],
                ["Config MD5", p.configMd5 && <code className="text-xs">{p.configMd5}</code>],
                ["Last provisioned", dateTime(p.lastSuccessUnix)],
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
              <Field label="Address" className="flex-1">
                <Input value={server} onChange={(e) => setServer(e.target.value)} placeholder="host:port" />
              </Field>
              <Button type="submit" variant="primary" busy={start.isPending} >
                {server.trim() ? "Connect" : "Discover"}
              </Button>
            </form>
            <p className="mb-2 mt-4 text-xs font-semibold uppercase tracking-wide text-slate-500">Discovered</p>
            {p.discovered.length === 0 ? (
              <Empty>No servers found</Empty>
            ) : (
              <ul className="divide-y divide-slate-100 dark:divide-slate-800">
                {p.discovered.map((d) => (
                  <li key={d.address} className="flex items-center justify-between gap-3 py-2">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{d.name}</p>
                      <p className="truncate text-xs text-slate-500">
                        {d.address} · {d.source !== DiscoverySource.UNSPECIFIED && label(discoverySourceNames, d.source, DiscoverySource[d.source])}
                      </p>
                    </div>
                    <Button size="sm" onClick={() => start.mutate({ server: d.address })}>
                      Connect
                    </Button>
                  </li>
                ))}
              </ul>
            )}
            <div className="mt-4 border-t border-slate-100 pt-4 dark:border-slate-800">
              <Button onClick={() => skip.mutate({})}>Skip Provisioning</Button>
            </div>
          </Card>
        </div>

        {p.phase === ProvisionPhase.AWAITING_USER && (
          <Card title="Select User">
            <div className="mb-4 max-w-xs">
              <Field label="PIN">
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
