import { Code, ConnectError } from "@connectrpc/connect";
import { useEffect, useRef, useState } from "react";

export type StreamStatus = "connecting" | "live" | "reconnecting";

/// Runs a server-streaming call for as long as the component is mounted,
/// reconnecting with backoff when it drops (the phone restarting, the
/// network going away).  `open` starts the call; `onMessage` gets each
/// message.  Changing `deps` restarts the stream.
export function useStream<T>(
  open: (signal: AbortSignal) => AsyncIterable<T>,
  onMessage: (msg: T) => void,
  deps: unknown[] = [],
): StreamStatus {
  const [status, setStatus] = useState<StreamStatus>("connecting");
  const handler = useRef(onMessage);
  handler.current = onMessage;
  const opener = useRef(open);
  opener.current = open;

  useEffect(() => {
    const abort = new AbortController();
    let delay = 500;
    (async () => {
      while (!abort.signal.aborted) {
        try {
          for await (const msg of opener.current(abort.signal)) {
            if (abort.signal.aborted) return;
            setStatus("live");
            delay = 500;
            handler.current(msg);
          }
        } catch (e) {
          if (abort.signal.aborted) return;
          if (e instanceof ConnectError && e.code === Code.Unauthenticated) return;
        }
        if (abort.signal.aborted) return;
        setStatus("reconnecting");
        await new Promise((r) => setTimeout(r, delay));
        delay = Math.min(delay * 2, 8000);
      }
    })();
    return () => abort.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return status;
}

/// The latest message of a stream, as state.
export function useStreamState<T>(
  open: (signal: AbortSignal) => AsyncIterable<T>,
  deps: unknown[] = [],
): [T | undefined, StreamStatus] {
  const [value, setValue] = useState<T>();
  const status = useStream(open, setValue, deps);
  return [value, status];
}
