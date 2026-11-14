import { Link, Outlet } from "@tanstack/react-router";
import { useState } from "react";
import { Logo } from "../App";
import { AuthService } from "../gen/dphone/v1/auth_pb";
import { setSession, useSession } from "../lib/session";
import { StatusProvider, useStatus } from "../lib/status";
import { client } from "../lib/transport";
import { Icons } from "./icons";
import { Alert, StreamBadge, cx } from "./ui";

const nav = [
  { to: "/", label: "Dashboard", icon: Icons.dashboard },
  { to: "/calls", label: "Calls", icon: Icons.phone },
  { to: "/accounts", label: "Accounts", icon: Icons.user },
  { to: "/contacts", label: "Contacts", icon: Icons.users },
  { to: "/network", label: "Network", icon: Icons.network },
  { to: "/provisioning", label: "Provisioning", icon: Icons.cloud },
  { to: "/settings", label: "Phone settings", icon: Icons.sliders },
  { to: "/apps", label: "Apps", icon: Icons.apps },
  { to: "/firmware", label: "Firmware", icon: Icons.chip },
  { to: "/logs", label: "Logs", icon: Icons.logs },
  { to: "/screen", label: "Screen", icon: Icons.screen },
  { to: "/system", label: "System", icon: Icons.power },
] as const;

function ThemeToggle() {
  const [dark, setDark] = useState(document.documentElement.classList.contains("dark"));
  return (
    <button
      type="button"
      aria-label={dark ? "Use the light theme" : "Use the dark theme"}
      className="rounded-lg p-2 text-slate-500 hover:bg-slate-100 dark:text-slate-400 dark:hover:bg-slate-800"
      onClick={() => {
        const next = !dark;
        document.documentElement.classList.toggle("dark", next);
        localStorage.setItem("theme", next ? "dark" : "light");
        setDark(next);
      }}
    >
      {dark ? <Icons.sun /> : <Icons.moon />}
    </button>
  );
}

function Shell() {
  const { status, stream } = useStatus();
  const session = useSession();
  const [open, setOpen] = useState(false);
  const sys = status?.system;
  const defaultPassword = session.status === "authenticated" && session.info.defaultPassword;

  async function logout() {
    try {
      await client(AuthService).logout({});
    } finally {
      setSession({ status: "anonymous", info: session.status === "authenticated" ? { ...session.info, authenticated: false, csrfToken: "" } : undefined });
    }
  }

  return (
    <div className="min-h-screen lg:pl-60">
      <aside
        className={cx(
          "fixed inset-y-0 left-0 z-40 flex w-60 flex-col border-r border-slate-200 bg-white transition-transform dark:border-slate-800 dark:bg-slate-900 lg:translate-x-0",
          open ? "translate-x-0" : "-translate-x-full",
        )}
      >
        <div className="flex items-center gap-3 px-5 py-4">
          <Logo className="size-9" />
          <div className="min-w-0">
            <p className="font-semibold leading-tight">Open DPhone</p>
            <p className="truncate text-xs text-slate-500 dark:text-slate-400">{sys ? `${sys.model} · ${sys.version}` : " "}</p>
          </div>
        </div>
        <nav className="flex-1 space-y-0.5 overflow-y-auto px-3 py-2">
          {nav.map((n) => (
            <Link
              key={n.to}
              to={n.to}
              onClick={() => setOpen(false)}
              activeOptions={{ exact: n.to === "/" }}
              className="flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
              activeProps={{ className: "!bg-brand-50 !text-brand-800 dark:!bg-brand-900/40 dark:!text-brand-200" }}
            >
              <n.icon />
              {n.label}
            </Link>
          ))}
        </nav>
        <div className="border-t border-slate-200 p-3 dark:border-slate-800">
          <button type="button" onClick={logout} className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800">
            <Icons.logout />
            Log out
          </button>
        </div>
      </aside>
      {open && <div className="fixed inset-0 z-30 bg-slate-950/40 lg:hidden" onClick={() => setOpen(false)} />}

      <header className="sticky top-0 z-20 flex items-center gap-3 border-b border-slate-200 bg-white/85 px-4 py-2.5 backdrop-blur dark:border-slate-800 dark:bg-slate-950/85 lg:px-8">
        <button type="button" aria-label="Menu" className="rounded-lg p-2 text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800 lg:hidden" onClick={() => setOpen(true)}>
          <Icons.menu />
        </button>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">{sys?.hostname || "Phone"}</p>
          <p className="truncate text-xs text-slate-500 dark:text-slate-400">{status?.network?.ipv4Address || " "}</p>
        </div>
        <StreamBadge status={stream} />
        <ThemeToggle />
      </header>

      <main className="mx-auto max-w-6xl px-4 py-6 lg:px-8">
        <div className="mb-4 space-y-3 empty:hidden">
          {defaultPassword && (
            <Alert title="The administrator password is still the factory default.">
              Change it under <Link to="/system" className="font-medium underline">System</Link>.
            </Alert>
          )}
          {status?.configError && <Alert tone="red" title="The last configuration merge failed.">{status.configError}</Alert>}
          {status?.restartNeeded && <Alert tone="blue" title="A changed setting takes effect after a restart." />}
          {sys?.simulated && <Alert tone="blue" title="This is the workstation simulator, not a phone." />}
        </div>
        <Outlet />
      </main>
    </div>
  );
}

export function Layout() {
  return (
    <StatusProvider>
      <Shell />
    </StatusProvider>
  );
}
