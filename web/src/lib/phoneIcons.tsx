// Key legends, drawn after the print on the phones (24 x 24 box).

import type { ReactNode } from "react";

const RECEIVER = "M6.2 2.6C4 3 2.9 5 3.4 7.3 5 14 10.4 19.6 17 21c2.2.5 4.2-.7 4.5-2.8l.2-1.7-4.9-1.9-1.7 2.3c-3.1-1.3-6.1-4.4-7.3-7.6l2.3-1.7L8 2.6z";

export function PhoneIcon({ name, color }: { name: string; color: string }): ReactNode {
  const stroke = { fill: "none", stroke: color, strokeLinecap: "round" as const, strokeLinejoin: "round" as const };
  switch (name) {
    case "check":
      return <path d="M4 13l5.5 5.5L20 6" {...stroke} strokeWidth={4} />;
    case "x":
      return <path d="M5 5l14 14M19 5L5 19" {...stroke} strokeWidth={4.5} />;
    case "up":
      return <path d="M5 16l7-7 7 7" {...stroke} strokeWidth={4} />;
    case "down":
      return <path d="M5 8l7 7 7-7" {...stroke} strokeWidth={4} />;
    case "left":
      return <path d="M16 5l-7 7 7 7" {...stroke} strokeWidth={4} />;
    case "right":
      return <path d="M8 5l7 7-7 7" {...stroke} strokeWidth={4} />;
    case "minus":
      return <path d="M4 12h16" {...stroke} strokeWidth={4} />;
    case "plus":
      return <path d="M4 12h16M12 4v16" {...stroke} strokeWidth={4} />;
    case "msgs":
      // Envelope.
      return (
        <>
          <rect x={1} y={5} width={22} height={14} rx={1.5} fill={color} />
          <path d="M1.5 6l10.5 8L22.5 6" fill="none" stroke="#17181a" strokeWidth={1.6} />
        </>
      );
    case "xfer":
      // Handset, arrow, handset.
      return (
        <>
          <path d="M5.5 4Q0 12 5.5 20M23 4Q17.5 12 23 20" {...stroke} strokeWidth={3.6} />
          <path d="M8 12h7.5M13 8.5l3.5 3.5-3.5 3.5" {...stroke} strokeWidth={2.4} />
        </>
      );
    case "redial":
      // Handset with a returning arrow.
      return (
        <>
          <path d={RECEIVER} fill={color} transform="translate(-1 3) scale(.86)" />
          <path d="M15 3.5a4.5 4.5 0 1 1-1 6" {...stroke} strokeWidth={2.2} />
          <path d="M12.5 8l1.3 2.3 2.5-.8" {...stroke} strokeWidth={2} />
        </>
      );
    case "hold":
      // Handset with pause bars.
      return (
        <>
          <path d={RECEIVER} fill={color} transform="translate(-1 3) scale(.86)" />
          <path d="M15.5 2.5v6M20 2.5v6" {...stroke} strokeWidth={2.6} strokeLinecap="butt" />
        </>
      );
    case "conf":
      // Three handsets.
      return <path d="M5.5 4Q0 12 5.5 20M13.5 4Q8 12 13.5 20M21.5 4Q16 12 21.5 20" {...stroke} strokeWidth={3.6} />;
    case "info":
      return (
        <>
          <circle cx={12} cy={12} r={10} fill={color} />
          <path d="M12 10.5v7" stroke="#17181a" strokeWidth={3.2} strokeLinecap="round" />
          <circle cx={12} cy={6.2} r={1.9} fill="#17181a" />
        </>
      );
    case "apps":
      // Open box.
      return (
        <>
          <path d="M4 10.5l8-3 8 3v8.5l-8 3-8-3z" fill={color} />
          <path d="M4 10.5L1 6.5l8-3 3 4M20 10.5l3-4-8-3-3 4" {...stroke} strokeWidth={1.8} />
          <path d="M12 13.5V22" stroke="#17181a" strokeWidth={1.4} />
        </>
      );
    case "dir":
      // Two people.
      return (
        <>
          <circle cx={7.5} cy={7} r={3.4} fill={color} />
          <circle cx={16.5} cy={7} r={3.4} fill={color} />
          <path d="M1.5 21q0-9 6-9t6 9zM10.5 21q0-9 6-9t6 9z" fill={color} />
        </>
      );
    case "menu":
      // Check mark over a list.
      return (
        <>
          <path d="M5 11.5h14M5 16h14M5 20.5h14" {...stroke} strokeWidth={2.6} strokeLinecap="butt" />
          <path d="M10 5l2.5 2.5L17.5 2" {...stroke} strokeWidth={2.2} />
        </>
      );
    case "status":
      // Circle with a bar.
      return (
        <>
          <circle cx={12} cy={12} r={9} {...stroke} strokeWidth={2.4} />
          <path d="M7 12h10" {...stroke} strokeWidth={3} />
        </>
      );
    case "headset":
      return (
        <>
          <path d="M5 14a7 7.5 0 0 1 14 0" {...stroke} strokeWidth={2.2} />
          <rect x={2.5} y={12} width={4.4} height={7.5} rx={2} fill={color} />
          <rect x={17.1} y={12} width={4.4} height={7.5} rx={2} fill={color} />
          <path d="M19.5 19.5q0 3-5.5 3" {...stroke} strokeWidth={1.8} />
          <circle cx={13} cy={22.5} r={1.6} fill={color} />
        </>
      );
    case "speaker":
      return (
        <>
          <path d="M2 9.5h3.5l4.5-4.5v14l-4.5-4.5H2z" fill={color} />
          <path d="M13 8.5q2.5 3.5 0 7M16.5 6q4.5 6 0 12M20 4q6 8 0 16" {...stroke} strokeWidth={1.9} />
        </>
      );
    case "mute":
      return (
        <>
          <rect x={8.5} y={2.5} width={7} height={12} rx={3.5} fill={color} />
          <path d="M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v3.5" {...stroke} strokeWidth={2} />
          <path d="M3.5 19.5l17-15" {...stroke} strokeWidth={2.2} />
        </>
      );
    default:
      return null;
  }
}
