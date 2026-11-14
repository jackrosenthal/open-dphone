import { useQuery } from "@connectrpc/connect-query";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Alert, Button, Card, PageHeader, Spinner, cx } from "../components/ui";
import { KeyAction, ScreenId, ScreenService, type Lamp } from "../gen/dphone/v1/screen_pb";
import { panels, type PanelKey } from "../lib/phones";
import { useStream } from "../lib/stream";
import { client, errorMessage } from "../lib/transport";

const lampColor: Record<string, string> = { red: "bg-red-500", green: "bg-emerald-500", amber: "bg-amber-400" };

/// Names for keys that carry no print on the phone.
function keyName(id: string): string {
  const n = id.replace(/\D/g, "");
  if (id.startsWith("sk")) return `Soft key ${n}`;
  if (id.startsWith("line")) return `Line key ${n}`;
  if (/^blf\d/.test(id)) return `Rapid dial key ${n}`;
  if (id === "blfleft") return "Previous page";
  if (id === "blfright") return "Next page";
  return id;
}

/// One mirrored display: draws each PNG the stream sends.
function Mirror({
  screen,
  width,
  height,
  scale,
  style,
  dim,
  onMeta,
}: {
  screen: ScreenId;
  width: number;
  height: number;
  scale: number;
  style?: CSSProperties;
  dim?: boolean;
  onMeta?: (lamps: Lamp[], backlight: boolean, offHook: boolean) => void;
}) {
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
    <div className="absolute overflow-hidden bg-black ring-2 ring-slate-950" style={{ ...style, width: width * scale, height: height * scale }}>
      {url && <img src={url} alt="" width={width * scale} height={height * scale} draggable={false} className={cx(dim && "opacity-60")} style={{ imageRendering: "pixelated" }} />}
      {status === "reconnecting" && <div className="absolute inset-0 flex items-center justify-center bg-black/70 text-xs text-amber-300">Connection lost</div>}
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
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLButtonElement || e.ctrlKey || e.metaKey || e.altKey) return;
      if (/^[0-9*#]$/.test(e.key)) return e.key;
      return ({ ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Enter: "select", Escape: "cancel", Backspace: "cancel", F1: "sk1", F2: "sk2", F3: "sk3", F4: "sk4" } as Record<string, string>)[e.key];
    };
    const down = (e: KeyboardEvent) => {
      const k = map(e);
      if (!k || e.repeat) return;
      e.preventDefault();
      press(k, KeyAction.DOWN);
    };
    const up = (e: KeyboardEvent) => {
      const k = map(e);
      if (!k) return;
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
  const { keys, screens, model, mirrorAvailable } = layout.data;
  const main = screens.find((s) => s.id === ScreenId.MAIN)!;
  const side = screens.find((s) => s.id === ScreenId.SIDE);
  const panel = panels[model];
  const lamp = (id?: string) => lamps.find((l) => l.key === id && l.color !== "off");

  function Key({ k, style, className }: { k: PanelKey; style?: CSSProperties; className?: string }) {
    const l = lamp(k.lamp);
    const release = () => {
      if (held.current === k.id) {
        held.current = undefined;
        press(k.id, KeyAction.UP);
      }
    };
    return (
      <button
        type="button"
        title={k.label ? undefined : keyName(k.id)}
        aria-label={k.label && /[A-Za-z0-9]/.test(k.label) ? undefined : keyName(k.id)}
        style={style}
        className={cx(
          "flex select-none flex-col items-center justify-center text-[11px] font-medium leading-none text-slate-100 active:brightness-125",
          k.shape === "rect" ? "rounded-md" : k.half === "left" ? "rounded-l-full" : k.half === "right" ? "rounded-r-full" : "rounded-full",
          // An unprinted key with a lamp lights as a whole.
          l && !k.label ? cx(lampColor[l.color], l.blink !== "solid" && `lamp-${l.blink}`) : "bg-slate-600 ring-1 ring-inset ring-slate-500",
          className,
        )}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          held.current = k.id;
          press(k.id, KeyAction.DOWN);
        }}
        onPointerUp={release}
        onPointerCancel={release}
        onClick={(e) => e.detail === 0 && press(k.id, KeyAction.TAP)}
      >
        {k.sub !== undefined ? (
          <>
            <span className="text-sm">{k.label}</span>
            <span className="h-2 text-[7px] tracking-widest text-slate-400">{k.sub}</span>
          </>
        ) : (
          k.label
        )}
        {l && k.label && <span className={cx("absolute right-1 top-1 size-1.5 rounded-full", lampColor[l.color], l.blink !== "solid" && `lamp-${l.blink}`)} />}
      </button>
    );
  }

  const placed = panel ? panel.keys.filter((k) => keys.includes(k.id)) : [];
  // Keys the phone reports that the drawing has no place for: listed
  // plainly rather than given an invented position.
  const unplaced = keys.filter((id) => !placed.some((k) => k.id === id));
  const mwi = lamp("msgs");

  return (
    <>
      <PageHeader
        title="Screen"
        actions={
          <Button
            variant={offHook ? "primary" : "secondary"}
            onClick={() =>
              client(ScreenService)
                .setHook({ offHook: !offHook })
                .catch((e) => setError(errorMessage(e)))
            }
          >
            {offHook ? "Replace handset" : "Lift handset"}
          </Button>
        }
      />
      <div className="space-y-4">
        {error && <Alert tone="red">{error}</Alert>}
        {!mirrorAvailable && <Alert tone="blue" title="This build has no screen picture. The keys still work." />}
        <Card className="overflow-x-auto">
          {panel && (
            <div className="relative mx-auto rounded-2xl bg-slate-800 ring-1 ring-slate-700" style={{ width: panel.width, height: panel.height }}>
              <div
                title="Message waiting"
                className={cx("absolute rounded-sm", mwi ? cx(lampColor[mwi.color], mwi.blink !== "solid" && `lamp-${mwi.blink}`) : "bg-slate-600")}
                style={{ left: panel.mwi.x, top: panel.mwi.y, width: panel.mwi.w, height: panel.mwi.h }}
              />
              <Mirror
                screen={ScreenId.MAIN}
                width={main.width}
                height={main.height}
                scale={panel.display.scale}
                dim={!backlight}
                style={{ left: panel.display.x, top: panel.display.y }}
                onMeta={(l, b, h) => {
                  setLamps(l);
                  setBacklight(b);
                  setOffHook(h);
                }}
              />
              {side && panel.side && <Mirror screen={ScreenId.SIDE} width={side.width} height={side.height} scale={1} style={{ left: panel.side.x, top: panel.side.y }} />}
              {panel.paper && <div className="absolute rounded-sm bg-slate-100" style={{ left: panel.paper.x, top: panel.paper.y, width: panel.paper.w, height: panel.paper.h }} />}
              {placed.map((k) => (
                <Key key={k.id} k={k} className="absolute" style={{ left: k.x, top: k.y, width: k.w, height: k.h }} />
              ))}
            </div>
          )}
          {unplaced.length > 0 && (
            <div className="mx-auto mt-4 flex max-w-xl flex-wrap gap-2">
              {unplaced.map((id) => (
                <Key key={id} k={{ id, label: id, x: 0, y: 0, w: 0, h: 0, shape: "rect" }} className="relative px-3 py-2" />
              ))}
            </div>
          )}
          <p className="mt-4 text-center text-xs text-slate-500 dark:text-slate-400">Keyboard: 0–9 * #, arrows, Enter for ✓, Esc for ✕, F1–F4 for the soft keys.</p>
        </Card>
      </div>
    </>
  );
}
