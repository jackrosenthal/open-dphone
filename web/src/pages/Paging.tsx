import { useMutation } from "@connectrpc/connect-query";
import { Alert, Badge, Button, Card } from "../components/ui";
import { PageActivity, PagingService, type PageGroup } from "../gen/dphone/v1/paging_pb";
import { useStreamState } from "../lib/stream";
import { client } from "../lib/transport";

function Address({ g }: { g: PageGroup }) {
  return (
    <span className="font-mono text-xs text-slate-500 dark:text-slate-400">
      {g.address}:{g.port}
    </span>
  );
}

/// Multicast paging, on the Calls page.  Absent when no page group is
/// configured.
export function PagingCard({ onError }: { onError: (e: unknown) => void }) {
  const [paging] = useStreamState((signal) => client(PagingService).watchPaging({}, { signal }));
  const start = useMutation(PagingService.method.startPage, { onError });
  const stop = useMutation(PagingService.method.stopPage, { onError });
  const snap = paging?.snapshot;
  if (!snap || (snap.listeners.length === 0 && snap.broadcasts.length === 0)) return null;

  const sending = snap.activity === PageActivity.SENDING;
  const receiving = snap.activity === PageActivity.RECEIVING;
  const name = snap.activeName || snap.activeGroup;
  // A group that is only listened to is listed without a button.
  const canPage = new Set(snap.broadcasts.map((g) => g.id));
  const listenOnly = snap.listeners.filter((g) => !canPage.has(g.id));
  const listening = new Set(snap.listeners.map((g) => g.id));

  return (
    <Card
      title="Paging"
      actions={
        (sending || receiving) && (
          <>
            <Badge tone={sending ? "green" : "blue"}>{sending ? `Paging ${name}` : `Page from ${name}`}</Badge>
            <Button size="sm" onClick={() => stop.mutate({})}>
              Stop
            </Button>
          </>
        )
      }
    >
      <div className="space-y-3">
        {snap.error && <Alert tone="red">{snap.error}</Alert>}
        {snap.activity === PageActivity.BLOCKED && <Alert title={`Page from ${name} blocked by an active call.`} />}
        <ul className="divide-y divide-slate-100 dark:divide-slate-800">
          {snap.broadcasts.map((g) => (
            <li key={`b${g.id}`} className="flex flex-wrap items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
              <div className="flex min-w-0 flex-wrap items-baseline gap-x-3">
                <span className="truncate text-sm font-medium">{g.name || g.id}</span>
                <Address g={g} />
              </div>
              <div className="flex items-center gap-2">
                {listening.has(g.id) && <Badge>Listening</Badge>}
                <Button size="sm" disabled={sending} busy={start.isPending && start.variables?.id === g.id} onClick={() => start.mutate({ id: g.id })}>
                  Page
                </Button>
              </div>
            </li>
          ))}
          {listenOnly.map((g) => (
            <li key={`l${g.id}`} className="flex flex-wrap items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0">
              <div className="flex min-w-0 flex-wrap items-baseline gap-x-3">
                <span className="truncate text-sm font-medium">{g.name || g.id}</span>
                <Address g={g} />
              </div>
              <Badge>Listening</Badge>
            </li>
          ))}
        </ul>
      </div>
    </Card>
  );
}
