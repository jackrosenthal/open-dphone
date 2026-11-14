import { Code, ConnectError, createClient, type Client, type Interceptor } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import type { DescService } from "@bufbuild/protobuf";
import { csrfToken, sessionExpired } from "./session";

const auth: Interceptor = (next) => async (req) => {
  const token = csrfToken();
  if (token) req.header.set("X-Csrf-Token", token);
  try {
    return await next(req);
  } catch (e) {
    if (e instanceof ConnectError && e.code === Code.Unauthenticated && !req.method.name.startsWith("Login")) {
      sessionExpired();
    }
    throw e;
  }
};

export const transport = createConnectTransport({
  baseUrl: "/",
  useBinaryFormat: true,
  interceptors: [auth],
});

const clients = new Map<DescService, unknown>();

/// A client for a service, created once.
export function client<T extends DescService>(service: T): Client<T> {
  let c = clients.get(service) as Client<T> | undefined;
  if (!c) {
    c = createClient(service, transport);
    clients.set(service, c);
  }
  return c;
}

export function errorMessage(e: unknown): string {
  if (e instanceof ConnectError) return e.rawMessage || e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}
