// A small set of line icons (24x24, stroke), drawn for this project.

import type { ReactNode } from "react";

function Icon({ children, className = "size-5" }: { children: ReactNode; className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

type P = { className?: string };

export const Icons = {
  dashboard: (p: P) => (
    <Icon {...p}>
      <rect x="3" y="3" width="7" height="9" rx="1.5" />
      <rect x="14" y="3" width="7" height="5" rx="1.5" />
      <rect x="14" y="12" width="7" height="9" rx="1.5" />
      <rect x="3" y="16" width="7" height="5" rx="1.5" />
    </Icon>
  ),
  phone: (p: P) => (
    <Icon {...p}>
      <path d="M6 3h3l2 5-2.5 1.5a11 11 0 0 0 6 6L16 13l5 2v3a2 2 0 0 1-2 2A16 16 0 0 1 4 5a2 2 0 0 1 2-2z" />
    </Icon>
  ),
  user: (p: P) => (
    <Icon {...p}>
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21a8 8 0 0 1 16 0" />
    </Icon>
  ),
  users: (p: P) => (
    <Icon {...p}>
      <circle cx="9" cy="8" r="3.5" />
      <path d="M2 20a7 7 0 0 1 14 0" />
      <path d="M16 4.5a3.5 3.5 0 0 1 0 7M18 14a7 7 0 0 1 4 6" />
    </Icon>
  ),
  network: (p: P) => (
    <Icon {...p}>
      <rect x="9" y="2" width="6" height="6" rx="1" />
      <rect x="2" y="16" width="6" height="6" rx="1" />
      <rect x="16" y="16" width="6" height="6" rx="1" />
      <path d="M12 8v4M5 16v-2a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v2" />
    </Icon>
  ),
  cloud: (p: P) => (
    <Icon {...p}>
      <path d="M7 18a5 5 0 1 1 1.5-9.8A6 6 0 0 1 20 10.5 3.8 3.8 0 0 1 18.5 18z" />
    </Icon>
  ),
  sliders: (p: P) => (
    <Icon {...p}>
      <path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0" />
      <circle cx="16" cy="6" r="2" />
      <circle cx="10" cy="12" r="2" />
      <circle cx="18" cy="18" r="2" />
    </Icon>
  ),
  apps: (p: P) => (
    <Icon {...p}>
      <rect x="3" y="3" width="7" height="7" rx="1.5" />
      <rect x="14" y="3" width="7" height="7" rx="1.5" />
      <rect x="3" y="14" width="7" height="7" rx="1.5" />
      <rect x="14" y="14" width="7" height="7" rx="1.5" />
    </Icon>
  ),
  chip: (p: P) => (
    <Icon {...p}>
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4" />
    </Icon>
  ),
  logs: (p: P) => (
    <Icon {...p}>
      <path d="M5 4h14M5 9h14M5 14h9M5 19h6" />
    </Icon>
  ),
  terminal: (p: P) => (
    <Icon {...p}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M7 9l3 3-3 3M13 15h4" />
    </Icon>
  ),
  screen: (p: P) => (
    <Icon {...p}>
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <path d="M8 20h8M12 16v4" />
    </Icon>
  ),
  power: (p: P) => (
    <Icon {...p}>
      <path d="M12 3v9" />
      <path d="M6.3 7a8 8 0 1 0 11.4 0" />
    </Icon>
  ),
  sun: (p: P) => (
    <Icon {...p}>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M5 19l1.5-1.5M17.5 6.5 19 5" />
    </Icon>
  ),
  moon: (p: P) => (
    <Icon {...p}>
      <path d="M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z" />
    </Icon>
  ),
  menu: (p: P) => (
    <Icon {...p}>
      <path d="M4 6h16M4 12h16M4 18h16" />
    </Icon>
  ),
  logout: (p: P) => (
    <Icon {...p}>
      <path d="M15 4h3a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-3M10 8l-4 4 4 4M6 12h10" />
    </Icon>
  ),
  check: (p: P) => (
    <Icon {...p}>
      <path d="M5 12.5 10 17.5 19 7" />
    </Icon>
  ),
  x: (p: P) => (
    <Icon {...p}>
      <path d="M6 6l12 12M18 6 6 18" />
    </Icon>
  ),
  warn: (p: P) => (
    <Icon {...p}>
      <path d="M12 3 2.5 20h19z" />
      <path d="M12 10v4M12 17v.5" />
    </Icon>
  ),
  plus: (p: P) => (
    <Icon {...p}>
      <path d="M12 5v14M5 12h14" />
    </Icon>
  ),
  trash: (p: P) => (
    <Icon {...p}>
      <path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" />
    </Icon>
  ),
};
