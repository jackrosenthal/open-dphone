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
        <Alert tone="red" title="The nvdata partition could not be mounted.">
          It has not been changed. The phone is running on default settings, and nothing saved now is kept. A reboot tries the mount again.
        </Alert>
        <ConfirmButton
          label="Erase nvdata"
          title="Erase nvdata"
          message="Every setting, account, contact, key, and app on the phone is deleted, and so is its MAC address."
          confirmLabel="Erase"
          requireText={model}
          onConfirm={() => erase.mutateAsync({ confirm: model })}
        />
      </div>
    </>
  );
}
