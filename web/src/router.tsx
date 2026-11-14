import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { Layout } from "./components/Layout";
import { Accounts } from "./pages/Accounts";
import { Apps } from "./pages/Apps";
import { Calls } from "./pages/Calls";
import { Contacts } from "./pages/Contacts";
import { Dashboard } from "./pages/Dashboard";
import { Firmware } from "./pages/Firmware";
import { Logs } from "./pages/Logs";
import { Network } from "./pages/Network";
import { Provisioning } from "./pages/Provisioning";
import { Screen } from "./pages/Screen";
import { Settings } from "./pages/Settings";
import { System } from "./pages/System";

const root = createRootRoute({ component: Layout });
const page = <P extends string>(path: P, component: () => React.JSX.Element) => createRoute({ getParentRoute: () => root, path, component });

const routeTree = root.addChildren([
  page("/", Dashboard),
  page("/calls", Calls),
  page("/accounts", Accounts),
  page("/contacts", Contacts),
  page("/network", Network),
  page("/provisioning", Provisioning),
  page("/settings", Settings),
  page("/apps", Apps),
  page("/firmware", Firmware),
  page("/logs", Logs),
  page("/screen", Screen),
  page("/system", System),
]);

export const router = createRouter({ routeTree, defaultPreload: false });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
