export function duration(secs: number | bigint): string {
  let s = Number(secs);
  const d = Math.floor(s / 86400);
  s -= d * 86400;
  const h = Math.floor(s / 3600);
  s -= h * 3600;
  const m = Math.floor(s / 60);
  s -= m * 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}:${pad(m)}:${pad(s)}`;
  return `${m}:${pad(s)}`;
}

export function dateTime(unix: number | bigint): string {
  const n = Number(unix);
  if (!n) return "";
  return new Date(n * 1000).toLocaleString();
}

export function bytes(n: number | bigint): string {
  const v = Number(n);
  if (v >= 1 << 20) return `${(v / (1 << 20)).toFixed(1)} MiB`;
  if (v >= 1 << 10) return `${(v / (1 << 10)).toFixed(1)} KiB`;
  return `${v} B`;
}
