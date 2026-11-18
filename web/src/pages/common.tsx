import { useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { Alert, Badge } from "../components/ui";
import { RegistrationState, type AccountStatus } from "../gen/opendeskphone/v1/accounts_pb";
import { CallState } from "../gen/opendeskphone/v1/calls_pb";
import { errorMessage } from "../lib/transport";

export function RegistrationBadge({ a }: { a: AccountStatus }) {
  switch (a.state) {
    case RegistrationState.REGISTERED:
      return <Badge tone="green">Registered{a.onAlternateHost ? " (backup)" : ""}</Badge>;
    case RegistrationState.REGISTERING:
      return <Badge tone="blue">Registering</Badge>;
    case RegistrationState.FAILED:
      return <Badge tone="red">Failed{a.code ? ` · ${a.code}` : ""}</Badge>;
    case RegistrationState.DISABLED:
      return <Badge>Disabled</Badge>;
    default:
      return <Badge tone="amber">Unregistered</Badge>;
  }
}

export function callStateLabel(s: CallState): string {
  return (
    {
      [CallState.UNSPECIFIED]: "Unknown",
      [CallState.DIALING]: "Dialing",
      [CallState.PROCEEDING]: "Calling",
      [CallState.RINGBACK]: "Ringing",
      [CallState.RINGING]: "Incoming",
      [CallState.ACTIVE]: "Connected",
      [CallState.HELD]: "On Hold",
      [CallState.TRANSFERRING]: "Transferring",
      [CallState.ENDED]: "Ended",
    } as Record<number, string>
  )[s];
}

export function ErrorNote({ error }: { error: unknown }): ReactNode {
  if (!error) return null;
  return <Alert tone="red">{errorMessage(error)}</Alert>;
}

/// Refetch every unary query (after a write; the application is small).
export function useRefresh() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries();
}
