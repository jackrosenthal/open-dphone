import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { lazy, Suspense } from "react";
import { Layout } from "./components/Layout";
import { Accounts } from "./pages/Accounts";
import { Apps } from "./pages/Apps";
import { Bluetooth } from "./pages/Bluetooth";
import { Calls } from "./pages/Calls";
import { Contacts } from "./pages/Contacts";
import { Dashboard } from "./pages/Dashboard";
import { Logs } from "./pages/Logs";
import { Network } from "./pages/Network";
import { Provisioning } from "./pages/Provisioning";
import { Screen } from "./pages/Screen";
import { Preferences } from "./pages/Settings";
import { System } from "./pages/System";

// The terminal emulator is large; it loads only when the Shell page opens.
const LazyShell = lazy(() => import("./pages/Shell"));
function Shell() {
  return (
    <Suspense fallback={null}>
      <LazyShell />
    </Suspense>
  );
}

const root = createRootRoute({ component: Layout });
const page = <P extends string>(path: P, component: () => React.JSX.Element) => createRoute({ getParentRoute: () => root, path, component });

const routeTree = root.addChildren([
  page("/", Dashboard),
  page("/calls", Calls),
  page("/accounts", Accounts),
  page("/contacts", Contacts),
  page("/network", Network),
  page("/provisioning", Provisioning),
  page("/preferences", Preferences),
  page("/bluetooth", Bluetooth),
  page("/apps", Apps),
  page("/logs", Logs),
  page("/screen", Screen),
  page("/shell", Shell),
  page("/system", System),
]);

export const router = createRouter({ routeTree, defaultPreload: false });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
