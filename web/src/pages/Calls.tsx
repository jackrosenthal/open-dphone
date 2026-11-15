import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useEffect, useState, type FormEvent } from "react";
import { Badge, Button, Card, ConfirmButton, Empty, Field, Input, PageHeader, Select, Toggle, td, th } from "../components/ui";
import { CallDirection, CallLogKind, CallState, CallsService, type Call } from "../gen/dphone/v1/calls_pb";
import { dateTime, duration } from "../lib/format";
import { codecNames, label } from "../lib/labels";
import { useStatus } from "../lib/status";
import { PagingCard } from "./Paging";
import { ErrorNote, callStateLabel } from "./common";

function CallRow({ call, onError, elapsed }: { call: Call; onError: (e: unknown) => void; elapsed: number }) {
  const opts = { onError };
  const answer = useMutation(CallsService.method.answer, opts);
  const hangup = useMutation(CallsService.method.hangup, opts);
  const hold = useMutation(CallsService.method.hold, opts);
  const resume = useMutation(CallsService.method.resume, opts);
  const transfer = useMutation(CallsService.method.transfer, opts);
  const dtmf = useMutation(CallsService.method.sendDtmf, opts);
  const [target, setTarget] = useState("");
  const [digits, setDigits] = useState("");
  const live = call.state !== CallState.ENDED;

  return (
    <li className="py-4 first:pt-0 last:pb-0">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate font-medium">{call.remote?.name || call.remote?.number || "Unknown"}</p>
          <p className="text-xs text-slate-500 dark:text-slate-400">
            {call.remote?.name ? `${call.remote.number} · ` : ""}
            {call.direction === CallDirection.INCOMING ? "Incoming" : "Outgoing"} · Line {call.account + 1}
            {call.codec && <> · {label(codecNames, call.codec)}</>}
            {call.srtp ? " · SRTP" : ""}
            {call.durationSecs !== undefined && call.state !== CallState.ENDED ? ` · ${duration(Number(call.durationSecs) + elapsed)}` : ""}
            {call.endCause ? ` · ${call.endCause}` : ""}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Badge tone={call.state === CallState.ACTIVE ? "green" : call.state === CallState.ENDED ? "gray" : "blue"}>{callStateLabel(call.state)}</Badge>
          {call.state === CallState.RINGING && (
            <Button size="sm" variant="primary" onClick={() => answer.mutate({ call: call.id })}>
              Answer
            </Button>
          )}
          {call.state === CallState.ACTIVE && (
            <Button size="sm" onClick={() => hold.mutate({ call: call.id })}>
              Hold
            </Button>
          )}
          {call.state === CallState.HELD && call.heldLocally && (
            <Button size="sm" onClick={() => resume.mutate({ call: call.id })}>
              Resume
            </Button>
          )}
          {live && (
            <Button size="sm" variant="danger" onClick={() => hangup.mutate({ call: call.id })}>
              Hang Up
            </Button>
          )}
        </div>
      </div>
      {(call.state === CallState.ACTIVE || call.state === CallState.HELD) && (
        <div className="mt-3 flex flex-wrap gap-3">
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (target.trim()) transfer.mutate({ call: call.id, to: { case: "target", value: target.trim() } }, { onSuccess: () => setTarget("") });
            }}
          >
            <Input className="w-40" placeholder="Transfer to" value={target} onChange={(e) => setTarget(e.target.value)} aria-label="Transfer to" />
            <Button size="sm" type="submit">
              Transfer
            </Button>
          </form>
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (digits) dtmf.mutate({ call: call.id, digits }, { onSuccess: () => setDigits("") });
            }}
          >
            <Input className="w-32" placeholder="DTMF digits" value={digits} onChange={(e) => setDigits(e.target.value.toUpperCase().replace(/[^0-9*#A-D]/g, ""))} aria-label="DTMF digits" />
            <Button size="sm" type="submit">
              Send
            </Button>
          </form>
        </div>
      )}
    </li>
  );
}

export function Calls() {
  const { status, elapsed } = useStatus();
  const [target, setTarget] = useState("");
  const [account, setAccount] = useState("");
  const [error, setError] = useState<unknown>();
  const opts = { onError: setError, onSuccess: () => setError(undefined) };
  const dial = useMutation(CallsService.method.dial, opts);
  const mute = useMutation(CallsService.method.setMute, opts);
  const dnd = useMutation(CallsService.method.setDnd, opts);
  const forward = useMutation(CallsService.method.setForward, opts);
  const clear = useMutation(CallsService.method.clearCallLog, opts);
  const log = useQuery(CallsService.method.getCallLog, {});
  const snap = status?.calls;
  const [fwd, setFwd] = useState<string>();

  // The call log changes when calls end.
  const callCount = snap?.calls.length ?? 0;
  useEffect(() => {
    log.refetch();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [callCount, snap?.missedUnseen]);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!target.trim()) return;
    dial.mutate({ target: target.trim(), account: account === "" ? undefined : Number(account) }, { onSuccess: () => setTarget("") });
  }
  const forwardValue = fwd ?? snap?.features?.forwardAlways ?? "";

  return (
    <>
      <PageHeader title="Calls" />
      <div className="space-y-5">
        <ErrorNote error={error} />
        <div className="grid gap-5 lg:grid-cols-3">
          <Card title="Dial" className="lg:col-span-2">
            <form onSubmit={submit} className="flex flex-wrap items-end gap-3">
              <Field label="Number or SIP URI" className="min-w-48 flex-1">
                <Input value={target} onChange={(e) => setTarget(e.target.value)} inputMode="tel" />
              </Field>
              <Field label="Line">
                <Select value={account} onChange={(e) => setAccount(e.target.value)}>
                  <option value="">Default</option>
                  {status?.accounts.map((a) => (
                    <option key={a.index} value={a.index}>
                      {a.label}
                    </option>
                  ))}
                </Select>
              </Field>
              <Button type="submit" variant="primary" busy={dial.isPending}>
                Dial
              </Button>
            </form>
          </Card>
          <Card title="Call Settings">
            <div className="space-y-4">
              <Toggle label="Do Not Disturb" checked={!!snap?.features?.dnd} onChange={(v) => dnd.mutate({ dnd: v })} />
              <Toggle label="Mute" checked={!!snap?.audio?.muted} onChange={(v) => mute.mutate({ muted: v })} />
              <form
                className="flex items-end gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  forward.mutate({ kind: "always", target: forwardValue.trim() }, { onSuccess: () => setFwd(undefined) });
                }}
              >
                <Field label="Forward calls to" className="flex-1">
                  <Input value={forwardValue} onChange={(e) => setFwd(e.target.value)} placeholder="Off" />
                </Field>
                <Button type="submit">Save</Button>
              </form>
            </div>
          </Card>
        </div>

        <PagingCard onError={setError} />

        <Card title="Active Calls">
          {!snap || snap.calls.length === 0 ? (
            <Empty>No active calls</Empty>
          ) : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {snap.calls.map((c) => (
                <CallRow key={c.id} call={c} onError={setError} elapsed={elapsed} />
              ))}
            </ul>
          )}
        </Card>

        <Card
          title="Call History"
          actions={
            (log.data?.entries.length ?? 0) > 0 && (
              <ConfirmButton size="sm" variant="secondary" label="Clear" title="Clear call history?" confirmLabel="Clear" onConfirm={() => clear.mutateAsync({}).then(() => log.refetch())} />
            )
          }
        >
          {!log.data || log.data.entries.length === 0 ? (
            <Empty>No calls</Empty>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr>
                    <th className={th}>Type</th>
                    <th className={th}>Name</th>
                    <th className={th}>Time</th>
                    <th className={th}>Duration</th>
                    <th className={th} />
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                  {log.data.entries.map((e) => (
                    <tr key={String(e.id)}>
                      <td className={td}>
                        <Badge tone={e.kind === CallLogKind.MISSED ? "red" : e.kind === CallLogKind.PLACED ? "blue" : "green"}>
                          {e.kind === CallLogKind.MISSED ? "Missed" : e.kind === CallLogKind.PLACED ? "Outgoing" : "Incoming"}
                        </Badge>
                      </td>
                      <td className={td}>
                        <span className="font-medium">{e.party?.name || e.party?.number}</span>
                        {e.party?.name && <span className="ml-2 text-slate-500">{e.party.number}</span>}
                      </td>
                      <td className={td}>{dateTime(e.startUnix)}</td>
                      <td className={td}>{e.durationSecs ? duration(e.durationSecs) : ""}</td>
                      <td className={`${td} text-right`}>
                        <Button size="sm" onClick={() => dial.mutate({ target: e.party?.number ?? "" })}>
                          Call
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Card>
      </div>
    </>
  );
}
