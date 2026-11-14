import { Alert, Card, Empty, td, th } from "../components/ui";
import { ExpansionService } from "../gen/dphone/v1/expansion_pb";
import { useStreamState } from "../lib/stream";
import { client } from "../lib/transport";

/// Attached expansion modules, on the Dashboard.  Absent while expansion
/// modules are off in the configuration and none is attached.
export function ExpansionCard() {
  const [expansion] = useStreamState((signal) => client(ExpansionService).watchExpansion({}, { signal }));
  const snap = expansion?.snapshot;
  if (!snap || (!snap.enabled && snap.modules.length === 0 && !snap.error)) return null;
  return (
    <Card title="Expansion modules">
      <div className="space-y-3">
        {snap.error && <Alert tone="red">{snap.error}</Alert>}
        {snap.modules.length === 0 ? (
          <Empty>None</Empty>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={th}>Module</th>
                  <th className={th}>Version</th>
                  <th className={th}>Address</th>
                  <th className={th}>Key page</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                {snap.modules.map((m) => (
                  <tr key={m.node}>
                    <td className={td}>{m.node + 1}</td>
                    <td className={td}>{m.version}</td>
                    <td className={`${td} font-mono text-xs`}>{m.address}</td>
                    <td className={td}>{m.page + 1}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </Card>
  );
}
