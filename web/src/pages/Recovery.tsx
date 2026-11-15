import { useMutation } from "@connectrpc/connect-query";
import { useState } from "react";
import { Alert, ConfirmButton, PageHeader } from "../components/ui";
import { SystemService } from "../gen/dphone/v1/system_pb";
import { ErrorNote } from "./common";

/// Shown in place of the dashboard when the phone could not mount nvdata.
export function Recovery({ model }: { model: string }) {
  const [error, setError] = useState<unknown>();
  const opts = { onError: setError };
  const erase = useMutation(SystemService.method.eraseNvdata, opts);
  return (
    <>
      <PageHeader title="Recovery" />
      <div className="max-w-2xl space-y-5">
        <ErrorNote error={error} />
        <Alert tone="red" title="Could not mount the nvdata partition.">
          The phone is running with default settings, and changes will not be saved. Reboot to try again.
        </Alert>
        <ConfirmButton
          label="Erase nvdata"
          title="Erase nvdata?"
          message="This deletes all settings, accounts, contacts, keys, and apps, and the MAC address."
          confirmLabel="Erase"
          requireText={model}
          onConfirm={() => erase.mutateAsync({ confirm: model })}
        />
      </div>
    </>
  );
}
