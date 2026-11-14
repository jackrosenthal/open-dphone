import { useEffect, useRef, useState } from "react";
import { Alert, Button, Card, Input, PageHeader, Select, Toggle, cx } from "../components/ui";
import { LogLevel, LogsService, type LogEntry } from "../gen/dphone/v1/logs_pb";
import { useStream } from "../lib/stream";
import { client } from "../lib/transport";

const MAX_LINES = 3000;

const levelStyle: Record<number, string> = {
  [LogLevel.ERROR]: "text-red-600 dark:text-red-400",
  [LogLevel.WARN]: "text-amber-600 dark:text-amber-400",
  [LogLevel.INFO]: "text-sky-700 dark:text-sky-400",
  [LogLevel.DEBUG]: "text-slate-500",
  [LogLevel.TRACE]: "text-slate-400",
};

function line(e: LogEntry): string {
  const t = new Date(Number(e.unixMs));
  return `${t.toISOString()} ${LogLevel[e.level]} ${e.target}: ${e.message}`;
}

export function Logs() {
  const [level, setLevel] = useState<LogLevel>(LogLevel.DEBUG);
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [filter, setFilter] = useState("");
  const [follow, setFollow] = useState(true);
  const [paused, setPaused] = useState(false);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const box = useRef<HTMLDivElement>(null);
  const lastSeq = useRef(0n);

  useEffect(() => {
    setEntries([]);
    lastSeq.current = 0n;
  }, [level]);

  const status = useStream(
    (signal) => client(LogsService).tailLogs({ backlog: 500, minLevel: level }, { signal }),
    (msg) => {
      // A reconnect replays the backlog; keep only what is new.
      const fresh = msg.entries.filter((e) => e.seq > lastSeq.current);
      if (fresh.length === 0 || pausedRef.current) return;
      lastSeq.current = fresh[fresh.length - 1].seq;
      setEntries((old) => {
        const next = old.concat(fresh);
        return next.length > MAX_LINES ? next.slice(next.length - MAX_LINES) : next;
      });
    },
    [level],
  );

  useEffect(() => {
    if (follow && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [entries, follow]);

  const q = filter.trim().toLowerCase();
  const shown = q ? entries.filter((e) => e.message.toLowerCase().includes(q) || e.target.toLowerCase().includes(q)) : entries;

  function download() {
    const url = URL.createObjectURL(new Blob([shown.map(line).join("\n") + "\n"], { type: "text/plain" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = "open-dphone.log";
    a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <>
      <PageHeader title="Logs" />
      {status === "reconnecting" && (
        <div className="mb-4">
          <Alert tone="red" title="Connection lost. New lines are not arriving." />
        </div>
      )}
      <Card>
        <div className="mb-3 flex flex-wrap items-center gap-3">
          <Select className="!w-auto" value={level} onChange={(e) => setLevel(Number(e.target.value))} aria-label="Least severe level shown">
            <option value={LogLevel.ERROR}>Errors</option>
            <option value={LogLevel.WARN}>Warnings and up</option>
            <option value={LogLevel.INFO}>Information and up</option>
            <option value={LogLevel.DEBUG}>Debug and up</option>
            <option value={LogLevel.TRACE}>Everything</option>
          </Select>
          <Input className="max-w-xs" placeholder="Filter" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter" />
          <Toggle label="Follow" checked={follow} onChange={setFollow} />
          <div className="flex-1" />
          <span className="text-xs tabular-nums text-slate-500">{shown.length} lines</span>
          <Button size="sm" onClick={() => setPaused(!paused)}>
            {paused ? "Resume" : "Pause"}
          </Button>
          <Button size="sm" onClick={() => setEntries([])}>
            Clear
          </Button>
          <Button size="sm" onClick={download}>
            Download
          </Button>
        </div>
        <div ref={box} className="h-[60vh] overflow-auto rounded-lg bg-slate-50 p-3 font-mono text-xs leading-5 ring-1 ring-inset ring-slate-200 dark:bg-slate-950 dark:ring-slate-800" onWheel={() => box.current && setFollow(box.current.scrollTop + box.current.clientHeight >= box.current.scrollHeight - 24)}>
          {shown.length === 0 ? (
            <p className="text-slate-400">{q ? "No matches" : "None"}</p>
          ) : (
            shown.map((e) => (
              <div key={String(e.seq)} className="whitespace-pre-wrap break-words">
                <span className="text-slate-400">{new Date(Number(e.unixMs)).toLocaleTimeString(undefined, { hour12: false })}</span>{" "}
                <span className={cx("font-semibold", levelStyle[e.level])}>{(LogLevel[e.level] ?? "").padEnd(5)}</span> <span className="text-slate-500">{e.target}</span>{" "}
                <span>{e.message}</span>
              </div>
            ))
          )}
        </div>
      </Card>
    </>
  );
}
