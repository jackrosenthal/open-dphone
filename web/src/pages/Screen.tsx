import { useQuery } from "@connectrpc/connect-query";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Alert, Button, Card, PageHeader, Spinner, StreamBadge, cx } from "../components/ui";
import { KeyAction, ScreenId, ScreenService, type Lamp } from "../gen/dphone/v1/screen_pb";
import { useStream } from "../lib/stream";
import { client, errorMessage } from "../lib/transport";

const labels: Record<string, ReactNode> = {
  up: "▲",
  down: "▼",
  left: "◀",
  right: "▶",
  select: "✓",
  cancel: "✕",
  vup: "Vol +",
  vdn: "Vol −",
  mute: "Mute",
  spkr: "Speaker",
  hdst: "Headset",
  hold: "Hold",
  xfer: "Transfer",
  redial: "Redial",
  msgs: "Msgs",
  conf: "Conf",
  info: "Info",
  status: "Status",
  dir: "Contacts",
  apps: "Apps",
  menu: "Menu",
  blfleft: "◀ Page",
  blfright: "Page ▶",
};
const letters: Record<string, string> = { "2": "ABC", "3": "DEF", "4": "GHI", "5": "JKL", "6": "MNO", "7": "PQRS", "8": "TUV", "9": "WXYZ" };

const lampColor: Record<string, string> = { red: "bg-red-500", green: "bg-emerald-500", amber: "bg-amber-400" };

/// One mirrored display: draws each PNG the stream sends.
function Mirror({ screen, width, height, scale, onMeta }: { screen: ScreenId; width: number; height: number; scale: number; onMeta?: (lamps: Lamp[], backlight: boolean, offHook: boolean) => void }) {
  const [url, setUrl] = useState<string>();
  const last = useRef<string>(undefined);
  const status = useStream(
    (signal) => client(ScreenService).watchScreen({ screen, maxFps: 8 }, { signal }),
    (msg) => {
      onMeta?.(msg.lamps, msg.backlightOn, msg.offHook);
      if (msg.png.length > 0) {
        const next = URL.createObjectURL(new Blob([msg.png as BlobPart], { type: "image/png" }));
        setUrl(next);
        if (last.current) URL.revokeObjectURL(last.current);
        last.current = next;
      }
    },
    [screen],
  );
  useEffect(() => () => void (last.current && URL.revokeObjectURL(last.current)), []);
  return (
    <div className="relative shrink-0 overflow-hidden rounded-md bg-black ring-4 ring-slate-800" style={{ width: width * scale, height: height * scale }}>
      {url ? <img src={url} alt="Phone screen" width={width * scale} height={height * scale} draggable={false} style={{ imageRendering: "pixelated" }} /> : <div className="flex size-full items-center justify-center text-xs text-slate-500">No picture</div>}
      <div className="absolute right-1 top-1 rounded bg-black/50 px-1">{status !== "live" && <StreamBadge status={status} />}</div>
    </div>
  );
}

export function Screen() {
  const layout = useQuery(ScreenService.method.getScreenLayout, {});
  const [lamps, setLamps] = useState<Lamp[]>([]);
  const [backlight, setBacklight] = useState(true);
  const [offHook, setOffHook] = useState(false);
  const [error, setError] = useState("");
  const held = useRef<string>(undefined);

  const press = (key: string, action: KeyAction) =>
    client(ScreenService)
      .pressKey({ key, action })
      .then(() => setError(""))
      .catch((e) => setError(errorMessage(e)));

  // The computer keyboard works as a keypad while this page is open.
  useEffect(() => {
    const map = (e: KeyboardEvent): string | undefined => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.ctrlKey || e.metaKey || e.altKey) return;
      if (/^[0-9*#]$/.test(e.key)) return e.key;
      return ({ ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Enter: "select", Escape: "cancel", Backspace: "cancel", F1: "sk1", F2: "sk2", F3: "sk3", F4: "sk4" } as Record<string, string>)[e.key];
    };
    const down = (e: KeyboardEvent) => {
      const k = map(e);
      if (!k || e.repeat) return;
      if (e.target instanceof HTMLButtonElement && e.key === "Enter") return;
      e.preventDefault();
      press(k, KeyAction.DOWN);
    };
    const up = (e: KeyboardEvent) => {
      const k = map(e);
      if (!k) return;
      if (e.target instanceof HTMLButtonElement && e.key === "Enter") return;
      e.preventDefault();
      press(k, KeyAction.UP);
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, []);

  if (layout.error) return <Alert tone="red">{errorMessage(layout.error)}</Alert>;
  if (!layout.data) return <Spinner />;
  const { keys, screens, skin, model, mirrorAvailable } = layout.data;
  const has = (k: string) => keys.includes(k);
  const main = screens.find((s) => s.id === ScreenId.MAIN)!;
  const side = screens.find((s) => s.id === ScreenId.SIDE);
  const scale = skin === "color" ? 1 : skin === "large_gray" ? 1.5 : 2;
  const lamp = (k: string) => lamps.find((l) => l.key === k && l.color !== "off");

  function Key({ k, className, children }: { k: string; className?: string; children?: ReactNode }) {
    if (!has(k)) return null;
    const l = lamp(k);
    const release = () => {
      if (held.current === k) {
        held.current = undefined;
        press(k, KeyAction.UP);
      }
    };
    return (
      <button
        type="button"
        title={k}
        aria-label={typeof (children ?? labels[k]) === "string" ? undefined : k}
        className={cx(
          "relative flex min-h-9 select-none items-center justify-center rounded-md bg-slate-700 px-2 py-1.5 text-xs font-medium text-slate-100 shadow-sm ring-1 ring-inset ring-slate-600 transition active:translate-y-px active:bg-slate-600",
          className,
        )}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          held.current = k;
          press(k, KeyAction.DOWN);
        }}
        onPointerUp={release}
        onPointerCancel={release}
        onClick={(e) => e.detail === 0 && press(k, KeyAction.TAP)}
      >
        {children ?? labels[k] ?? k}
        {l && <span className={cx("absolute right-1 top-1 size-2 rounded-full shadow", lampColor[l.color], l.blink !== "solid" && `lamp-${l.blink}`)} />}
      </button>
    );
  }

  const lineKeys = keys.filter((k) => k.startsWith("line"));
  const blfKeys = keys.filter((k) => /^blf\d+$/.test(k));
  const functionKeys = ["msgs", "hold", "xfer", "redial", "conf", "info", "status", "dir", "apps", "menu", "blfleft", "blfright"].filter(has);

  return (
    <>
      <PageHeader
        title="Screen"
        description={`A live picture of the ${model}'s display, with a working keypad. Digits, arrows, Enter, Esc, and F1 to F4 on your keyboard work too.`}
        actions={
          <Button
            variant={offHook ? "primary" : "secondary"}
            onClick={() =>
              client(ScreenService)
                .setHook({ offHook: !offHook })
                .catch((e) => setError(errorMessage(e)))
            }
          >
            {offHook ? "Handset lifted: hang up" : "Lift handset"}
          </Button>
        }
      />
      <div className="space-y-4">
        {error && <Alert tone="red">{error}</Alert>}
        {!mirrorAvailable && <Alert tone="blue" title="No screen picture is available in this build; the keys still work." />}
        <Card className="overflow-x-auto">
          <div className="mx-auto w-fit rounded-3xl bg-slate-900 p-5 shadow-xl ring-1 ring-slate-700">
            <div className="flex items-start justify-center gap-3">
              <div className="flex flex-col justify-around gap-2 self-stretch py-1">
                {lineKeys.map((k) => (
                  <Key key={k} k={k} className="w-12">
                    {`L${k.slice(4)}`}
                  </Key>
                ))}
              </div>
              <div>
                <div className={cx("transition-opacity", !backlight && "opacity-60")}>
                  <Mirror
                    screen={ScreenId.MAIN}
                    width={main.width}
                    height={main.height}
                    scale={scale}
                    onMeta={(l, b, h) => {
                      setLamps(l);
                      setBacklight(b);
                      setOffHook(h);
                    }}
                  />
                </div>
                <div className="mt-2 grid grid-cols-4 gap-2">
                  {[1, 2, 3, 4].map((n) => (
                    <Key key={n} k={`sk${n}`} className="!min-h-6 !py-0.5">
                      <span className="text-[10px] text-slate-400">F{n}</span>
                    </Key>
                  ))}
                </div>
              </div>
              {(side || blfKeys.length > 0) && (
                <div className="flex gap-2">
                  {side && <Mirror screen={ScreenId.SIDE} width={side.width} height={side.height} scale={1} />}
                  {blfKeys.length > 0 && (
                    <div className="flex flex-col justify-between gap-1">
                      {blfKeys.map((k) => (
                        <Key key={k} k={k} className="!min-h-0 w-12 flex-1 !py-0.5">
                          {`B${k.slice(3)}`}
                        </Key>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>

            <div className="mt-5 flex flex-wrap items-start justify-center gap-6">
              <div className="grid w-44 grid-cols-2 gap-2">
                {functionKeys.map((k) => (
                  <Key key={k} k={k} />
                ))}
              </div>
              <div className="grid grid-cols-3 gap-2">
                <span />
                <Key k="up" />
                <span />
                <Key k="left" />
                <Key k="select" className="bg-brand-700 ring-brand-600" />
                <Key k="right" />
                <Key k="cancel" />
                <Key k="down" />
                <span />
              </div>
              <div className="grid w-44 grid-cols-3 gap-2">
                {["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"].map((d) => (
                  <Key key={d} k={d} className="flex-col !py-1">
                    <span className="text-base leading-none">{d}</span>
                    <span className="h-2.5 text-[8px] leading-none tracking-widest text-slate-400">{letters[d] ?? ""}</span>
                  </Key>
                ))}
              </div>
              <div className="grid w-40 grid-cols-2 gap-2">
                <Key k="vdn" />
                <Key k="vup" />
                <Key k="mute" />
                <Key k="hdst" />
                <Key k="spkr" className="col-span-2" />
              </div>
            </div>
          </div>
        </Card>
      </div>
    </>
  );
}
