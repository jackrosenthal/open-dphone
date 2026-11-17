// Key legends, drawn after the print on the phones (24 x 24 box).

import type { ReactNode } from "react";

/// A handset as the phones print it: a C-shaped handle opening to the
/// right, with the earpiece and the mouthpiece as two caps at its ends,
/// each set apart from the handle by a narrow gap.  20 high and 10 wide,
/// centered on the origin; place it with a transform.
function Handset({ color, transform }: { color: string; transform: string }) {
  return (
    <g transform={transform}>
      <path d="M1.7-8.05C-4.63-8.05-4.63 8.05 1.7 8.05" fill="none" stroke={color} strokeWidth={4.5} />
      <rect x={2.5} y={-10} width={2.7} height={5.4} rx={1.1} fill={color} />
      <rect x={2.5} y={4.6} width={2.7} height={5.4} rx={1.1} fill={color} />
    </g>
  );
}

/// The handset leaning back, earpiece up and to the left, as on the hold
/// and redial keys.
const LEANING = "translate(9.6 14.2) rotate(-45) scale(.9)";

/// `children` crossed by the line `slash`, with a narrow gap cut out of
/// them on each side of the line.
function Slashed({ id, slash, color, children }: { id: string; slash: string; color: string; children: ReactNode }) {
  return (
    <>
      <mask id={id} maskUnits="userSpaceOnUse" x={-2} y={-2} width={28} height={28}>
        <rect x={-2} y={-2} width={28} height={28} fill="#fff" />
        <path d={slash} stroke="#000" strokeWidth={4.6} strokeLinecap="round" />
      </mask>
      <g mask={`url(#${id})`}>{children}</g>
      <path d={slash} stroke={color} strokeWidth={2} strokeLinecap="round" />
    </>
  );
}

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
    case "asterisk":
      // The dial pad's star: six square-ended arms.
      return <path d="M12 2v20M3.34 7l17.32 10M3.34 17l17.32-10" fill="none" stroke={color} strokeWidth={3.4} />;
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
          <Handset color={color} transform="translate(3 12) scale(.9)" />
          <path d="M9.6 12h5.6M13 8.8l3.2 3.2-3.2 3.2" {...stroke} strokeWidth={2.2} />
          <Handset color={color} transform="translate(21 12) scale(.9)" />
        </>
      );
    case "redial":
      // The leaning handset, and a circular arrow turning counterclockwise
      // with the arrowhead at the top.
      return (
        <>
          <Handset color={color} transform={LEANING} />
          <path d="M14.4 6.06A4.2 4.2 0 1 0 19.8 3.56" {...stroke} strokeWidth={2} strokeLinecap="butt" />
          <path d="M16.15 3.3L19.8.9v4.8z" fill={color} />
        </>
      );
    case "hold":
      // The leaning handset with pause bars.
      return (
        <>
          <Handset color={color} transform={LEANING} />
          <path d="M16.3 2.8v6.4M20.1 2.8v6.4" fill="none" stroke={color} strokeWidth={2.2} />
        </>
      );
    case "conf":
      // Three handsets.
      return (
        <>
          <Handset color={color} transform="translate(3.8 12) scale(.8)" />
          <Handset color={color} transform="translate(12 12) scale(.8)" />
          <Handset color={color} transform="translate(20.2 12) scale(.8)" />
        </>
      );
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
    // New family (D60, D62, D65): drawn face on.
    case "headset":
      return (
        <>
          <path d="M5.4 11.5v-1a6.6 7.4 0 0 1 13.2 0v1" {...stroke} strokeWidth={1.5} />
          <rect x={3.6} y={9.4} width={3.6} height={7.8} rx={1.3} fill={color} />
          <rect x={16.8} y={9.4} width={3.6} height={7.8} rx={1.3} fill={color} />
          <path d="M3.1 11.5L1 13.3l2.1 1.8zM20.9 11.5l2.1 1.8-2.1 1.8z" fill={color} />
          <path d="M18.6 17.2q0 2.9-3.3 3.5" {...stroke} strokeWidth={1.4} />
          <circle cx={14.2} cy={20.8} r={1.25} fill="none" stroke={color} strokeWidth={1.1} />
        </>
      );
    case "speaker":
      return (
        <>
          <path d="M2.2 10.7l1.5-1.9h2.4v6.4H3.7l-1.5-1.9z" fill={color} />
          <path d="M6.1 10.1Q7.2 4.8 9.6 4.8a4.4 7.2 0 0 1 0 14.4Q7.2 19.2 6.1 13.9z" {...stroke} strokeWidth={1.5} />
          <path d="M15.6 6.6a8.6 8.6 0 0 1 0 10.8M19 4a12 12 0 0 1 0 16" {...stroke} strokeWidth={1.8} />
        </>
      );
    case "mute":
      return (
        <Slashed id="mute-cut" slash="M3.6 14.6L20.6 4.4" color={color}>
          <path d="M8.7 11.2V5.2q0-3.2 3.3-3.2t3.3 3.2v6z" fill={color} />
          <rect x={8.7} y={12.1} width={6.6} height={2.5} fill={color} />
          <path d="M6.9 13.2q0 4.6 5.1 4.6t5.1-4.6M8.3 21.1q3.7 1.9 7.4 0" {...stroke} strokeWidth={1.6} />
        </Slashed>
      );
    // Old family (D40, D45, D50, D70): drawn turned, as on those phones.
    case "headset-turned":
      return (
        <>
          <path d="M5.3 14.6C4.1 7.3 10.1 2.4 15.6 5.4" {...stroke} strokeWidth={1.5} />
          <circle cx={7.6} cy={16} r={3.5} fill={color} />
          <circle cx={16.4} cy={8.9} r={3} fill={color} />
          <path d="M8.1 19.9q4.3 2.4 9.2-.6" {...stroke} strokeWidth={1.5} />
        </>
      );
    case "speaker-turned":
      return (
        <>
          <ellipse cx={9.6} cy={14.6} rx={4.4} ry={5.2} transform="rotate(-35 9.6 14.6)" {...stroke} strokeWidth={1.5} />
          <circle cx={6.7} cy={17.3} r={2.1} {...stroke} strokeWidth={1.5} />
          <path d="M9.1 5.6a9 9 0 0 1 9.4 9.4M10.2 1.9a12.6 12.6 0 0 1 11.9 12" {...stroke} strokeWidth={1.6} />
        </>
      );
    case "mute-turned":
      return (
        <Slashed id="mute-turned-cut" slash="M4.4 14.8L19.8 5.4" color={color}>
          <rect x={9.3} y={2.4} width={5.4} height={10.8} rx={2.7} fill={color} />
          <path d="M7.3 10.6q0 5.9 4.7 5.9t4.7-5.9M12 16.5v2.6M8.7 20.6q3.3 1.8 6.6 0" {...stroke} strokeWidth={1.6} />
        </Slashed>
      );
    default:
      return null;
  }
}
