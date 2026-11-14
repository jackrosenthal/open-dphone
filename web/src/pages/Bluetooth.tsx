import { useMutation } from "@connectrpc/connect-query";
import { useState, type FormEvent } from "react";
import { Alert, Badge, Button, Card, ConfirmButton, Empty, Input, PageHeader, Spinner, Toggle } from "../components/ui";
import { BluetoothService, type BluetoothDevice, type BluetoothPairing } from "../gen/dphone/v1/bluetooth_pb";
import { useStatus } from "../lib/status";
import { useStreamState } from "../lib/stream";
import { client } from "../lib/transport";
import { ErrorNote } from "./common";

function Device({ d }: { d: BluetoothDevice }) {
  return (
    <div className="min-w-0">
      <p className="truncate text-sm font-medium">{d.name || <span className="font-mono">{d.address}</span>}</p>
      {d.name && <p className="truncate font-mono text-xs text-slate-500 dark:text-slate-400">{d.address}</p>}
    </div>
  );
}

/// The pairing question: compare the passkey, or type the device's PIN.
function Pairing({ p, onError }: { p: BluetoothPairing; onError: (e: unknown) => void }) {
  const [pin, setPin] = useState("");
  const answer = useMutation(BluetoothService.method.answerBluetoothPairing, { onError });
  const send = (accept: boolean) => answer.mutate({ address: p.address, accept, pin: accept && p.pinNeeded ? pin : undefined });
  function submit(e: FormEvent) {
    e.preventDefault();
    send(true);
  }
  return (
    <Card title={`Pair with ${p.name || p.address}`}>
      <form onSubmit={submit} className="flex flex-wrap items-end gap-x-6 gap-y-4">
        {p.passkey !== undefined && (
          <div>
            <p className="text-xs text-slate-500 dark:text-slate-400">Passkey on both devices</p>
            <p className="font-mono text-3xl font-bold tracking-widest tabular-nums">{String(p.passkey).padStart(6, "0")}</p>
          </div>
        )}
        {p.pinNeeded && (
          <label className="block">
            <span className="mb-1 block text-sm font-medium text-slate-700 dark:text-slate-300">PIN</span>
            <Input className="w-40 font-mono" value={pin} onChange={(e) => setPin(e.target.value)} maxLength={16} autoFocus autoComplete="off" />
          </label>
        )}
        <div className="flex gap-2">
          <Button type="submit" variant="primary" busy={answer.isPending} disabled={p.pinNeeded && pin === ""}>
            Pair
          </Button>
          <Button onClick={() => send(false)}>Cancel</Button>
        </div>
      </form>
    </Card>
  );
}

export function Bluetooth() {
  const { status } = useStatus();
  const [bt] = useStreamState((signal) => client(BluetoothService).watchBluetooth({}, { signal }));
  const [error, setError] = useState<unknown>();
  const opts = { onError: setError, onSuccess: () => setError(undefined) };
  const enable = useMutation(BluetoothService.method.setBluetoothEnabled, opts);
  const discover = useMutation(BluetoothService.method.setBluetoothDiscovery, opts);
  const pair = useMutation(BluetoothService.method.pairBluetoothDevice, opts);
  const connect = useMutation(BluetoothService.method.connectBluetoothDevice, opts);
  const disconnect = useMutation(BluetoothService.method.disconnectBluetoothDevice, opts);
  const remove = useMutation(BluetoothService.method.removeBluetoothDevice, opts);

  const snap = bt?.snapshot;
  const header = <PageHeader title="Bluetooth" />;
  if (status?.system && !status.system.hasBluetooth) {
    return (
      <>
        {header}
        <Empty>This model has no Bluetooth.</Empty>
      </>
    );
  }
  if (!snap) {
    return (
      <>
        {header}
        <Spinner />
      </>
    );
  }
  const on = snap.enabled && snap.powered;
  const paired = new Set(snap.paired.map((d) => d.address));
  const found = snap.found.filter((d) => !paired.has(d.address));

  return (
    <>
      {header}
      <div className="space-y-5">
        <ErrorNote error={error} />
        {!snap.present && <Alert tone="red" title="The Bluetooth controller did not answer." />}
        {snap.error && <Alert tone="red">{snap.error}</Alert>}

        <Card>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-3">
              <Toggle label="Bluetooth" checked={snap.enabled} disabled={!snap.present || enable.isPending} onChange={(v) => enable.mutate({ enabled: v })} />
              {snap.enabled && !snap.powered && <Badge tone="amber">Starting</Badge>}
            </div>
            {snap.address && <span className="font-mono text-xs text-slate-500 dark:text-slate-400">{snap.address}</span>}
          </div>
        </Card>

        {snap.pairing && <Pairing key={snap.pairing.address} p={snap.pairing} onError={setError} />}

        {on && (
          <div className="grid gap-5 lg:grid-cols-2">
            <Card title="Paired">
              {snap.paired.length === 0 ? (
                <Empty>None</Empty>
              ) : (
                <ul className="divide-y divide-slate-100 dark:divide-slate-800">
                  {snap.paired.map((d) => {
                    const connected = snap.connected === d.address;
                    return (
                      <li key={d.address} className="flex flex-wrap items-center justify-between gap-3 py-3 first:pt-0 last:pb-0">
                        <Device d={d} />
                        <div className="flex flex-wrap items-center gap-2">
                          {connected && <Badge tone="green">{snap.audioOpen ? "In call" : "Connected"}</Badge>}
                          {connected ? (
                            <Button size="sm" onClick={() => disconnect.mutate({})}>
                              Disconnect
                            </Button>
                          ) : (
                            <Button size="sm" busy={connect.isPending && connect.variables?.address === d.address} onClick={() => connect.mutate({ address: d.address })}>
                              Connect
                            </Button>
                          )}
                          <ConfirmButton size="sm" variant="secondary" label="Remove" title={`Remove ${d.name || d.address}`} confirmLabel="Remove" onConfirm={() => remove.mutateAsync({ address: d.address })} />
                        </div>
                      </li>
                    );
                  })}
                </ul>
              )}
            </Card>

            <Card
              title="Found"
              actions={
                <>
                  {snap.discovering && <Spinner className="size-4" />}
                  <Button size="sm" onClick={() => discover.mutate({ discovering: !snap.discovering })}>
                    {snap.discovering ? "Stop" : "Scan"}
                  </Button>
                </>
              }
            >
              {found.length === 0 ? (
                <Empty>None</Empty>
              ) : (
                <ul className="divide-y divide-slate-100 dark:divide-slate-800">
                  {found.map((d) => (
                    <li key={d.address} className="flex items-center justify-between gap-3 py-3 first:pt-0 last:pb-0">
                      <Device d={d} />
                      <div className="flex shrink-0 items-center gap-3">
                        {d.rssi !== 0 && <span className="text-xs tabular-nums text-slate-500 dark:text-slate-400">{d.rssi} dBm</span>}
                        <Button size="sm" busy={snap.pairingWith === d.address || (pair.isPending && pair.variables?.address === d.address)} disabled={!!snap.pairing || !!snap.pairingWith} onClick={() => pair.mutate({ address: d.address })}>
                          Pair
                        </Button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </Card>
          </div>
        )}
      </div>
    </>
  );
}
