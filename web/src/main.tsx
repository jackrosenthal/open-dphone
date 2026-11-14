import { TransportProvider } from "@connectrpc/connect-query";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./index.css";
import { transport } from "./lib/transport";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false, staleTime: 5_000 } },
});

const theme = localStorage.getItem("theme");
if (theme === "dark" || (!theme && matchMedia("(prefers-color-scheme: dark)").matches)) {
  document.documentElement.classList.add("dark");
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <TransportProvider transport={transport}>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </TransportProvider>
  </StrictMode>,
);
