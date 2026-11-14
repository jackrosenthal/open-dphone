import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITheme } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";
import { Button, PageHeader } from "../components/ui";
import { ShellService, type ShellExited } from "../gen/dphone/v1/shell_pb";
import { csrfToken } from "../lib/session";
import { client, errorMessage } from "../lib/transport";

// Input is gathered for this long, and sent in pieces of at most this
// size, so typing costs a request per burst and a paste a few requests.
const INPUT_DELAY_MS = 12;
const INPUT_CHUNK = 16 * 1024;
// The phone ends a shell whose page it has not heard from for 75 seconds.
const HEARTBEAT_MS = 25_000;
// 12 pt, as in a desktop terminal.
const FONT_SIZE = 16;

// The Wild Cherry color scheme; the terminal's own, in both UI themes.
const theme: ITheme = {
  background: "#1f1626",
  foreground: "#d9faff",
  cursor: "#dd00ff",
  cursorAccent: "#1f1626",
  selectionBackground: "#002731",
  selectionForeground: "#1f1626",
  black: "#000506",
  red: "#d94085",
  green: "#2ab250",
  yellow: "#ffd06e",
  blue: "#873bdb",
  magenta: "#ececec",
  cyan: "#c1b8b6",
  white: "#fff8dd",
  brightBlack: "#009cc9",
  brightRed: "#da6bab",
  brightGreen: "#f4dba5",
  brightYellow: "#eac066",
  brightBlue: "#2f8bb9",
  brightMagenta: "#ae636b",
  brightCyan: "#ff919d",
  brightWhite: "#e4838d",
};

function ended(e: ShellExited): string {
  if (e.reason === "idle") return "Closed after 30 minutes without input.";
  if (e.reason === "logged out") return "Closed: the session was logged out.";
  if (e.reason === "page gone") return "Closed: the phone lost contact with this page.";
  if (e.reason) return "Closed.";
  if (e.signal) return `The shell was ended by signal ${e.signal}.`;
  return e.exitCode === 0 ? "The shell exited." : `The shell exited with status ${e.exitCode}.`;
}

export default function Shell() {
  const box = useRef<HTMLDivElement>(null);
  // Each run is one shell; "Reconnect" starts the next.
  const [run, setRun] = useState(0);
  // Why the shell is over: its exit, or an error.
  const [over, setOver] = useState("");

  useEffect(() => {
    const host = box.current;
    if (!host) return;
    setOver("");
    // The UI's one monospace family (index.css).
    const fontFamily = getComputedStyle(document.documentElement).getPropertyValue("--font-mono").trim() || "monospace";
    const term = new Terminal({ cursorBlink: true, fontFamily, fontSize: FONT_SIZE, scrollback: 5000, theme });
    const fit = new FitAddon();
    term.loadAddon(fit);

    const shell = client(ShellService);
    const abort = new AbortController();
    const encoder = new TextEncoder();
    let id = "";
    let done = false;
    const queue: Uint8Array[] = [];
    let sending = false;
    let inputTimer: number | undefined;
    let resizeTimer: number | undefined;
    const finish = (why: string) => {
      if (done) return;
      done = true;
      term.options.cursorBlink = false;
      term.options.disableStdin = true;
      setOver(why);
    };

    // One write in flight at a time, so input arrives in order.
    async function flush() {
      if (sending || !id) return;
      sending = true;
      try {
        while (queue.length > 0 && !done) {
          let size = 0;
          let n = 0;
          while (n < queue.length && (n === 0 || size + queue[n].length <= INPUT_CHUNK)) size += queue[n++].length;
          const data = new Uint8Array(size);
          let at = 0;
          for (const part of queue.splice(0, n)) {
            data.set(part, at);
            at += part.length;
          }
          await shell.writeShell({ sessionId: id, data });
        }
      } catch (e) {
        if (!abort.signal.aborted) finish(errorMessage(e));
      } finally {
        sending = false;
      }
    }
    function input(bytes: Uint8Array) {
      if (done) return;
      for (let at = 0; at < bytes.length; at += INPUT_CHUNK) queue.push(bytes.subarray(at, at + INPUT_CHUNK));
      inputTimer ??= window.setTimeout(() => {
        inputTimer = undefined;
        void flush();
      }, INPUT_DELAY_MS);
    }
    term.onData((text) => input(encoder.encode(text)));
    // Mouse reports and other raw bytes, one character per byte.
    term.onBinary((text) => input(Uint8Array.from(text, (c) => c.charCodeAt(0) & 0xff)));
    term.onResize(({ cols, rows }) => {
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        if (id && !done) shell.resizeShell({ sessionId: id, cols, rows }).catch(() => {});
      }, 80);
    });
    // Ctrl+C copies when text is selected (and interrupts otherwise);
    // Ctrl+Shift+V pastes.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown" || !e.ctrlKey || e.altKey || e.metaKey) return true;
      const key = e.key.toLowerCase();
      if (key === "c" && term.hasSelection()) {
        document.execCommand("copy");
        term.clearSelection();
        e.preventDefault();
        return false;
      }
      // Leave it to the browser, whose paste event the terminal handles.
      if (key === "v" && e.shiftKey) return false;
      return true;
    });

    const heartbeat = window.setInterval(() => {
      if (id && !done && !sending && queue.length === 0) shell.writeShell({ sessionId: id, data: new Uint8Array() }).catch(() => {});
    }, HEARTBEAT_MS);
    // Leaving the page ends the shell at once.  A browser may keep a page
    // it navigated away from, and its connections, for a while; this
    // request is sent regardless.
    function leaving() {
      if (!id || done) return;
      fetch("/dphone.v1.ShellService/CloseShell", {
        method: "POST",
        keepalive: true,
        headers: { "Content-Type": "application/json", "X-Csrf-Token": csrfToken() ?? "" },
        body: JSON.stringify({ sessionId: id }),
      }).catch(() => {});
    }
    window.addEventListener("pagehide", leaving);
    const resized = new ResizeObserver(() => {
      if (term.element) fit.fit();
    });

    (async () => {
      // The cell size is measured when the terminal opens, so the font
      // has to be there first (it may be the embedded one, still loading).
      // Laying out text in the family makes the browser fetch the embedded
      // font only if the viewer has none of the families before it.
      const probe = document.createElement("div");
      probe.style.cssText = `position:absolute;visibility:hidden;font:${FONT_SIZE}px ${fontFamily}`;
      probe.innerHTML = "Ab<b>Ab</b>";
      host.appendChild(probe);
      void probe.offsetWidth;
      await document.fonts.ready.catch(() => {});
      probe.remove();
      if (abort.signal.aborted) return;
      term.open(host);
      fit.fit();
      resized.observe(host);
      term.focus();
      try {
        for await (const msg of shell.openShell({ cols: term.cols, rows: term.rows }, { signal: abort.signal })) {
          if (msg.event.case === "started") {
            id = msg.event.value.sessionId;
            void flush();
          } else if (msg.event.case === "output") {
            term.write(msg.event.value);
          } else if (msg.event.case === "exited") {
            finish(ended(msg.event.value));
          }
        }
        if (!abort.signal.aborted) finish("Connection to the phone lost.");
      } catch (e) {
        if (!abort.signal.aborted) finish(errorMessage(e));
      }
    })();

    return () => {
      leaving();
      done = true;
      // Dropping the stream ends the shell on the phone, too.
      abort.abort();
      window.clearInterval(heartbeat);
      window.clearTimeout(inputTimer);
      window.clearTimeout(resizeTimer);
      window.removeEventListener("pagehide", leaving);
      resized.disconnect();
      term.dispose();
    };
  }, [run]);

  return (
    <div className="relative h-full" style={{ background: theme.background }}>
      <PageHeader title="Shell" />
      <div ref={box} className="absolute inset-0" />
      {over && (
        <div className="absolute inset-x-0 bottom-0 z-10 flex items-center justify-center gap-3 bg-black/60 px-4 py-3 text-sm text-white backdrop-blur-sm">
          <span>{over}</span>
          <Button size="sm" onClick={() => setRun(run + 1)}>
            Reconnect
          </Button>
        </div>
      )}
    </div>
  );
}
