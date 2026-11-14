// The login session, held in memory only.  The cookie is HttpOnly; the
// CSRF token is fetched again (GetSession) after a page reload.

import { useSyncExternalStore } from "react";
import type { Session } from "../gen/dphone/v1/auth_pb";

export type SessionState =
  | { status: "loading" }
  | { status: "anonymous"; info?: Session }
  | { status: "authenticated"; info: Session };

let state: SessionState = { status: "loading" };
const listeners = new Set<() => void>();

export function getSession(): SessionState {
  return state;
}

export function setSession(next: SessionState) {
  state = next;
  listeners.forEach((l) => l());
}

export function csrfToken(): string {
  return state.status === "authenticated" ? state.info.csrfToken : "";
}

/// Called when the server says the session is gone.
export function sessionExpired() {
  if (state.status === "authenticated") {
    setSession({ status: "anonymous", info: { ...state.info, authenticated: false, csrfToken: "" } });
  }
}

export function useSession(): SessionState {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => state,
  );
}
