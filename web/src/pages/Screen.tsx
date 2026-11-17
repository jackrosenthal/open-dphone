import { useQuery } from "@connectrpc/connect-query";
import { useEffect, useRef, useState } from "react";
import { Alert, Card, PageHeader, Spinner } from "../components/ui";
import { KeyAction, ScreenId, ScreenService, type Lamp } from "../gen/dphone/v1/screen_pb";
import { PhoneIcon } from "../lib/phoneIcons";
import { archedRect, panels, type Panel, type PanelKey } from "../lib/phones";
import { useStream, type StreamStatus } from "../lib/stream";
import { client, errorMessage } from "../lib/transport";

const lampFill: Record<string, string> = { red: "#ef4444", green: "#22c55e", amber: "#f59e0b" };

/// Names for keys whose print is an icon or nothing.
function keyName(id: string): string {
  const n = id.replace(/\D/g, "");
  if (id.startsWith("sk")) return `Softkey ${n}`;
  if (id.startsWith("line")) return `Line key ${n}`;
  if (/^blf\d/.test(id)) return `Rapid dial key ${n}`;
  return (
    {
      blfleft: "Previous page",
      blfright: "Next page",
      hdst: "Headset",
      spkr: "Speaker",
      mute: "Mute",
      vdn: "Volume down",
      vup: "Volume up",
      select: "Select",
      cancel: "Cancel",
      up: "Up",
      down: "Down",
      left: "Left",
      right: "Right",
      msgs: "Messages",
      xfer: "Transfer",
      redial: "Redial",
      hold: "Hold",
      conf: "Conference",
      info: "Info",
      apps: "Apps",
      dir: "Contacts",
      menu: "Menu",
      status: "Status",
    }[id] ?? id
  );
}

type Meta = { lamps: Lamp[]; backlight: boolean; offHook: boolean };

/// The latest picture of one display, as an object URL.
function useMirror(screen: ScreenId, enabled: boolean, onMeta?: (m: Meta) => void): [string | undefined, StreamStatus] {
  const [url, setUrl] = useState<string>();
  const last = useRef<string>(undefined);
  const status = useStream(
    (signal) => (enabled ? client(ScreenService).watchScreen({ screen, maxFps: 8 }, { signal }) : (async function* () {})()),
    (msg) => {
      onMeta?.({ lamps: msg.lamps, backlight: msg.backlightOn, offHook: msg.offHook });
      if (msg.png.length > 0) {
        const next = URL.createObjectURL(new Blob([msg.png as BlobPart], { type: "image/png" }));
        setUrl(next);
        if (last.current) URL.revokeObjectURL(last.current);
        last.current = next;
      }
    },
    [screen, enabled],
  );
  useEffect(() => () => void (last.current && URL.revokeObjectURL(last.current)), []);
  return [url, status];
}

function Legend({ k }: { k: PanelKey }) {
  const ink = k.ink ?? "#f2f3f4";
  const l = k.legend;
  if (!l) return null;
  if ("icon" in l) {
    const s = k.size ?? 16;
    return (
      <g transform={`translate(${k.cx - s / 2} ${k.cy - s / 2}) scale(${s / 24})`}>
        <PhoneIcon name={l.icon} color={ink} />
      </g>
    );
  }
  if ("text" in l) {
    return (
      <text x={k.cx} y={k.cy} fill={ink} fontSize={k.size ?? 10} fontWeight={600} textAnchor="middle" dominantBaseline="central">
        {l.text}
      </text>
    );
  }
  return (
    <>
      {l.letters && (
        <text x={k.cx} y={k.cy - k.h * 0.27} fill={ink} fontSize={k.h * 0.23} fontWeight={700} textAnchor="middle" dominantBaseline="central">
          {l.letters}
        </text>
      )}
      <text x={k.cx} y={k.cy + (l.letters !== undefined ? k.h * 0.16 : 0)} fill={ink} fontSize={k.h * 0.52} fontWeight={700} textAnchor="middle" dominantBaseline="central">
        {l.digit}
      </text>
    </>
  );
}

function Phone({ panel, model, keys, sideSize }: { panel: Panel; model: string; keys: string[]; sideSize?: { width: number; height: number } }) {
  const [meta, setMeta] = useState<Meta>({ lamps: [], backlight: true, offHook: false });
  const [error, setError] = useState("");
  const held = useRef<string>(undefined);
  const [mainUrl, mainStatus] = useMirror(ScreenId.MAIN, true, setMeta);
  const [sideUrl] = useMirror(ScreenId.SIDE, !!panel.side && !!sideSize);

  const press = (key: string, action: KeyAction) =>
    client(ScreenService)
      .pressKey({ key, action })
      .then(() => setError(""))
      .catch((e) => setError(errorMessage(e)));
  const toggleHook = () =>
    client(ScreenService)
      .setHook({ offHook: !meta.offHook })
      .then(() => setError(""))
      .catch((e) => setError(errorMessage(e)));

  const lamp = (id?: string) => meta.lamps.find((l) => l.key === id && l.color !== "off");
  const [vx, vy, vw, vh] = panel.viewBox;
  const d = panel.display;
  const hs = panel.handset;
  const mwi = lamp("msgs");

  return (
    <>
      {error && (
        <div className="mb-4">
          <Alert tone="red">{error}</Alert>
        </div>
      )}
      <svg viewBox={`${vx} ${vy} ${vw} ${vh}`} className="mx-auto block h-auto w-full select-none" style={{ maxWidth: vw * panel.unit }} role="group" aria-label={`${model} front panel`}>
        <defs>
          <linearGradient id="gloss" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#fff" stopOpacity="0.16" />
            <stop offset="0.45" stopColor="#fff" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="handset" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stopColor="#fff" stopOpacity="0.02" />
            <stop offset="0.4" stopColor="#fff" stopOpacity="0.14" />
            <stop offset="1" stopColor="#000" stopOpacity="0.25" />
          </linearGradient>
          <filter id="lift" x="-30%" y="-10%" width="160%" height="120%">
            <feDropShadow dx="10" dy="12" stdDeviation="8" floodOpacity="0.45" />
          </filter>
        </defs>

        {panel.shapes.map((s, i) => (
          <g key={i}>
            <path d={s.d} fill={s.fill} stroke={s.stroke} opacity={s.opacity} />
            {s.gloss && <path d={s.d} fill="url(#gloss)" />}
          </g>
        ))}

        {/* Speaker grille in the cradle: vertical slots within an oval. */}
        {Array.from({ length: 11 }, (_, i) => {
          const g = panel.grille;
          const x = g.cx + ((i - 5) * g.rx) / 5.6;
          const half = g.ry * Math.sqrt(Math.max(0, 1 - ((x - g.cx) / g.rx) ** 2)) * 0.95;
          const w = g.rx / 9;
          // Each column is a long slot broken by a short bridge, offset
          // from its neighbor, with a round hole at one end.
          const gap = half * (i % 2 ? 0.25 : -0.2);
          return (
            <g key={i} stroke="#08090a" strokeWidth={w} strokeLinecap="round">
              <path d={`M${x} ${g.cy - half + w}V${g.cy + gap - w * 1.4}`} />
              <path d={`M${x} ${g.cy + gap + w * 1.4}V${g.cy + half - w}`} />
              <circle cx={x} cy={g.cy + gap} r={w * 0.2} />
            </g>
          );
        })}

        {/* Displays. */}
        <rect x={d.x} y={d.y} width={d.w} height={d.h} fill="#050506" />
        {mainUrl && <image href={mainUrl} x={d.x} y={d.y} width={d.w} height={d.h} preserveAspectRatio="none" opacity={meta.backlight ? 1 : 0.6} style={{ imageRendering: "pixelated" }} />}
        {mainStatus === "reconnecting" && (
          <text x={d.x + d.w / 2} y={d.y + d.h / 2} fill="#fbbf24" fontSize={d.h / 9} textAnchor="middle" dominantBaseline="central">
            Connection lost
          </text>
        )}
        {panel.side && (
          <>
            <rect x={panel.side.x} y={panel.side.y} width={panel.side.w} height={panel.side.h} fill="#050506" />
            {sideUrl && <image href={sideUrl} x={panel.side.x} y={panel.side.y} width={panel.side.w} height={panel.side.h} preserveAspectRatio="none" style={{ imageRendering: "pixelated" }} />}
          </>
        )}

        {/* Printed lines from keys to the display. */}
        {panel.lines.map(([x1, y1, x2, y2], i) => (
          <path key={i} d={`M${x1} ${y1}L${x2} ${y2}`} stroke={panel.lineColor} strokeWidth={0.9} />
        ))}

        {/* Message waiting lamp. */}
        <path d={panel.mwi.d} fill={panel.mwi.off}>
          <title>Message waiting</title>
        </path>
        {mwi && <path d={panel.mwi.d} fill={lampFill[mwi.color]} className={mwi.blink !== "solid" ? `lamp-${mwi.blink}` : undefined} pointerEvents="none" />}
        <path d={panel.mwi.d} fill="none" stroke="#00000040" strokeWidth={0.7} pointerEvents="none" />

        {/* Keys. */}
        {panel.keys
          .filter((k) => keys.includes(k.id))
          .map((k) => {
            const l = lamp(k.lamp);
            const box = { x: k.cx - k.w / 2, y: k.cy - k.h / 2, width: k.w, height: k.h, rx: k.r };
            const release = () => {
              if (held.current === k.id) {
                held.current = undefined;
                press(k.id, KeyAction.UP);
              }
            };
            return (
              <g
                key={k.id}
                role="button"
                aria-label={keyName(k.id)}
                className="cursor-pointer outline-none [&:active>.key-face]:brightness-150"
                onPointerDown={(e) => {
                  e.currentTarget.setPointerCapture(e.pointerId);
                  held.current = k.id;
                  press(k.id, KeyAction.DOWN);
                }}
                onPointerUp={release}
                onPointerCancel={release}
              >
                <title>{keyName(k.id)}</title>
                {k.arch ? (
                  <path className="key-face" d={archedRect(box.x, box.y, box.width, box.height, k.r, k.arch)} fill={k.fill} stroke="#00000066" strokeWidth={0.8} />
                ) : (
                  <rect className="key-face" {...box} fill={k.bare ? "transparent" : k.fill} stroke={k.bare ? "none" : "#00000066"} strokeWidth={0.8} />
                )}
                {l &&
                  (k.arch ? (
                    <path d={archedRect(box.x, box.y, box.width, box.height, k.r, k.arch)} fill={lampFill[l.color]} className={l.blink !== "solid" ? `lamp-${l.blink}` : undefined} />
                  ) : (
                    <rect {...box} fill={lampFill[l.color]} className={l.blink !== "solid" ? `lamp-${l.blink}` : undefined} />
                  ))}
                <Legend k={k} />
              </g>
            );
          })}

        {/* Handset, in its cradle; click to lift or replace it. */}
        <g
          role="button"
          aria-label={meta.offHook ? "Hang up handset" : "Pick up handset"}
          aria-pressed={meta.offHook}
          className="cursor-pointer outline-none"
          onClick={toggleHook}
          transform={meta.offHook ? `translate(${-(hs.w + 14)} ${-hs.h * 0.02})` : undefined}
          filter={meta.offHook ? "url(#lift)" : undefined}
        >
          <title>{meta.offHook ? "Hang up handset" : "Pick up handset"}</title>
          {/* Coiled cord from the mouthpiece end. */}
          <path
            d={`M${hs.x + hs.w * 0.5} ${hs.y + hs.h - 6}c0 ${hs.w * 0.3} ${-hs.w * 0.12} ${hs.w * 0.36} ${-hs.w * 0.3} ${hs.w * 0.36}`}
            fill="none"
            stroke="#141516"
            strokeWidth={hs.w * 0.085}
            strokeLinecap="round"
            strokeDasharray={`${hs.w * 0.022} ${hs.w * 0.016}`}
          />
          <rect x={hs.x} y={hs.y} width={hs.w} height={hs.h} rx={hs.w * 0.36} fill={hs.fill} stroke="#00000055" />
          <rect x={hs.x} y={hs.y} width={hs.w} height={hs.h} rx={hs.w * 0.36} fill="url(#handset)" />
          {/* The bevel along the edges; the handset is even along its length. */}
          <rect x={hs.x + hs.w * 0.08} y={hs.y + hs.w * 0.08} width={hs.w * 0.84} height={hs.h - hs.w * 0.16} rx={hs.w * 0.3} fill="none" stroke="#ffffff" strokeOpacity={0.07} strokeWidth={1.2} />
          {hs.print && (
            <text x={hs.x + hs.w * 0.5} y={hs.y + hs.h * 0.86} fill="#aeb4bc" fontSize={hs.w * 0.1} fontWeight={600} textAnchor="middle">
              {hs.print}
            </text>
          )}
        </g>
      </svg>
    </>
  );
}

export function Screen() {
  const layout = useQuery(ScreenService.method.getScreenLayout, {});
  const [error, setError] = useState("");

  if (layout.error) return <Alert tone="red">{errorMessage(layout.error)}</Alert>;
  if (!layout.data) return <Spinner />;
  const { screens, model, mirrorAvailable } = layout.data;
  const keys = layout.data.keys.map((k) => k.name);
  const side = screens.find((s) => s.id === ScreenId.SIDE);
  const panel = panels[model];
  // Keys the phone reports that the drawing has no place for: listed
  // plainly rather than given an invented position.
  const unplaced = panel ? keys.filter((id) => !panel.keys.some((k) => k.id === id)) : keys;
  const tap = (id: string) =>
    client(ScreenService)
      .pressKey({ key: id, action: KeyAction.TAP })
      .then(() => setError(""))
      .catch((e) => setError(errorMessage(e)));

  return (
    <>
      <PageHeader title="Screen" />
      <div className="space-y-4">
        {error && <Alert tone="red">{error}</Alert>}
        {!mirrorAvailable && <Alert tone="blue" title="Screen mirroring is unavailable." />}
        <Card>
          {panel && <Phone panel={panel} model={model} keys={keys} sideSize={side} />}
          {unplaced.length > 0 && (
            <div className="mx-auto mt-4 flex max-w-xl flex-wrap gap-2">
              {unplaced.map((id) => (
                <button key={id} type="button" className="rounded-md bg-slate-200 px-3 py-2 text-xs font-medium dark:bg-slate-700" onClick={() => tap(id)}>
                  {id}
                </button>
              ))}
            </div>
          )}
        </Card>
      </div>
    </>
  );
}
