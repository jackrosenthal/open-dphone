import { RouterProvider } from "@tanstack/react-router";
import { useEffect, useState, type FormEvent } from "react";
import { Alert, Button, Field, Input, Spinner } from "./components/ui";
import { AuthService } from "./gen/dphone/v1/auth_pb";
import { setSession, useSession } from "./lib/session";
import { client, errorMessage } from "./lib/transport";
import { router } from "./router";

export function App() {
  const session = useSession();

  useEffect(() => {
    client(AuthService)
      .getSession({})
      .then((r) => {
        const info = r.session!;
        setSession(info.authenticated ? { status: "authenticated", info } : { status: "anonymous", info });
      })
      .catch(() => setSession({ status: "anonymous" }));
  }, []);

  if (session.status === "loading") {
    return (
      <div className="flex h-screen items-center justify-center text-slate-400">
        <Spinner className="size-8" />
      </div>
    );
  }
  if (session.status === "anonymous") return <Login model={session.info?.model} hostname={session.info?.hostname} />;
  return <RouterProvider router={router} />;
}

function Logo({ className = "size-10" }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={className} aria-hidden="true">
      <rect width="32" height="32" rx="7" className="fill-brand-600" />
      <path d="M10 8h3.2l1.6 4.6-2 1.6a12 12 0 0 0 5 5l1.6-2 4.6 1.6V22a2 2 0 0 1-2 2A14 14 0 0 1 8 10a2 2 0 0 1 2-2z" fill="#fff" />
    </svg>
  );
}
export { Logo };

function Login({ model, hostname }: { model?: string; hostname?: string }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const r = await client(AuthService).login({ user: "admin", password });
      setSession({ status: "authenticated", info: r.session! });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center px-4">
      <div className="w-full max-w-sm">
        <div className="mb-8 flex flex-col items-center gap-3 text-center">
          <Logo className="size-14" />
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">Open DPhone</h1>
            <p className="text-sm text-slate-500 dark:text-slate-400">
              {model ? `Digium ${model}` : ""}
              {hostname ? ` · ${hostname}` : ""}
            </p>
          </div>
        </div>
        <form onSubmit={submit} className="space-y-4 rounded-xl bg-white p-6 shadow-sm ring-1 ring-slate-200 dark:bg-slate-900 dark:ring-slate-800">
          <Field label="Administrator password">
            <Input type="password" name="password" autoComplete="current-password" autoFocus required value={password} onChange={(e) => setPassword(e.target.value)} />
          </Field>
          {error && <Alert tone="red">{error}</Alert>}
          <Button type="submit" variant="primary" busy={busy} className="w-full">
            Log in
          </Button>
        </form>
      </div>
    </div>
  );
}
