// One WatchStatus stream for the whole application: the header, the
// dashboard, and several pages read the same live snapshot.

import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { StatusService, type StatusSnapshot } from "../gen/opendeskphone/v1/status_pb";
import { useStream, type StreamStatus } from "./stream";
import { client } from "./transport";

type Value = {
  status?: StatusSnapshot;
  stream: StreamStatus;
  /// Seconds since the snapshot arrived: add it to the snapshot's counters
  /// (uptime, call timers, the clock) so they run between updates.
  elapsed: number;
};

const Ctx = createContext<Value>({ stream: "connecting", elapsed: 0 });

export function StatusProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<StatusSnapshot>();
  const received = useRef(Date.now());
  const [elapsed, setElapsed] = useState(0);
  const stream = useStream(
    (signal) => client(StatusService).watchStatus({}, { signal }),
    (msg) => {
      received.current = Date.now();
      setElapsed(0);
      setStatus(msg.status);
    },
  );
  useEffect(() => {
    const t = setInterval(() => setElapsed(Math.floor((Date.now() - received.current) / 1000)), 1000);
    return () => clearInterval(t);
  }, []);
  return <Ctx.Provider value={{ status, stream, elapsed }}>{children}</Ctx.Provider>;
}

export function useStatus() {
  return useContext(Ctx);
}
