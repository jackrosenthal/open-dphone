import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useRef, useState, type ReactNode } from "react";
import { Alert, Badge, Button, Card, ConfirmButton, KeyValue, PageHeader, Spinner, Toggle } from "../components/ui";
import { FirmwareService, UpdatePhase } from "../gen/dphone/v1/firmware_pb";
import { bytes } from "../lib/format";
import { label, updatePhaseNames } from "../lib/labels";
import { useStatus } from "../lib/status";
import { upload } from "../lib/upload";
import { ErrorNote } from "./common";
import { settingOrigin } from "./Ssh";

function Bar({ fraction, label }: { fraction: number; label: ReactNode }) {
  return (
    <div>
      <div className="mb-1 flex justify-between text-sm">
        <span>{label}</span>
        <span className="tabular-nums text-slate-500">{Math.round(fraction * 100)}%</span>
      </div>
      <div className="h-2.5 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700" role="progressbar" aria-valuenow={Math.round(fraction * 100)} aria-valuemin={0} aria-valuemax={100}>
        <div className="h-full rounded-full bg-brand-600 transition-[width] duration-300" style={{ width: `${Math.min(100, fraction * 100)}%` }} />
      </div>
    </div>
  );
}

// Rough share of the whole job per phase, for one overall bar.
const span: Partial<Record<UpdatePhase, [number, number]>> = {
  [UpdatePhase.CHECKING]: [0, 0.03],
  [UpdatePhase.ERASING]: [0.03, 0.15],
  [UpdatePhase.WRITING_KERNEL]: [0.15, 0.2],
  [UpdatePhase.WRITING_ROOTFS]: [0.2, 0.75],
  [UpdatePhase.VERIFYING]: [0.75, 0.98],
  [UpdatePhase.COMMITTING]: [0.98, 1],
};

function ProvisionedFirmware() {
  const [error, setError] = useState<unknown>();
  const policy = useQuery(FirmwareService.method.getProvisionedFirmwarePolicy, {});
  const set = useMutation(FirmwareService.method.setProvisionedFirmwarePolicy, {
    onError: setError,
    onSuccess: () => {
      setError(undefined);
      policy.refetch();
    },
  });
  const ignore = policy.data?.ignore;
  const origin = settingOrigin(ignore);
  return (
    <Card title="Provisioned firmware">
      <div className="space-y-2">
        <ErrorNote error={error ?? policy.error} />
        <Toggle label="Ignore firmware provisioned from DPMA" checked={ignore?.value ?? false} disabled={!ignore || ignore.locked || set.isPending} onChange={(v) => set.mutate({ ignore: v })} />
        {origin && <p className="text-xs text-slate-500 dark:text-slate-400">{origin}</p>}
      </div>
    </Card>
  );
}

export function Firmware() {
  const { status } = useStatus();
  const fw = status?.firmware;
  const file = useRef<HTMLInputElement>(null);
  const [sending, setSending] = useState<number>();
  const [error, setError] = useState<unknown>();
  // The upload waiting for confirmation is not part of the phone's state
  // (nothing changes there until it is installed), so it is asked for.
  const pending = useQuery(FirmwareService.method.getFirmware, {});
  const opts = {
    onError: setError,
    onSuccess: () => {
      setError(undefined);
      pending.refetch();
    },
  };
  const install = useMutation(FirmwareService.method.installFirmware, opts);
  const discard = useMutation(FirmwareService.method.discardUpload, opts);
  const confirm = useMutation(FirmwareService.method.confirmTrial, opts);
  const rollback = useMutation(FirmwareService.method.rollback, opts);

  if (!fw) return <Spinner />;
  const up = pending.data?.status?.uploaded;
  const working = fw.phase !== UpdatePhase.IDLE && fw.phase !== UpdatePhase.COMPLETE && fw.phase !== UpdatePhase.FAILED;
  const [from, to] = span[fw.phase] ?? [0, 0];
  const overall = from + ((to - from) * fw.percent) / 100;

  async function send(f: File) {
    setError(undefined);
    setSending(0);
    try {
      await upload("/upload/firmware", f, setSending);
    } catch (e) {
      setError(e);
    } finally {
      await pending.refetch();
      setSending(undefined);
    }
  }

  return (
    <>
      <PageHeader title="Firmware" />
      <div className="space-y-5">
        <ErrorNote error={error} />
        {fw.trial === "pending" && (
          <Alert tone="blue" title={`This firmware is on trial. Starts left: ${fw.trialBootsLeft}.`}>
            <div className="mt-2 flex gap-2">
              <Button size="sm" onClick={() => confirm.mutate({})}>
                Keep this firmware
              </Button>
              <ConfirmButton size="sm" variant="secondary" label="Return to the previous firmware" title="Return to the previous firmware" confirmLabel="Return" onConfirm={() => rollback.mutateAsync({})} />
            </div>
          </Alert>
        )}
        {fw.trial === "rolled_back" && <Alert tone="red" title="The last update failed its trial and was rolled back.">{fw.rollbackReason}</Alert>}

        <div className="grid gap-5 lg:grid-cols-2">
          <Card title="Installed">
            <KeyValue
              rows={[
                ["Version", `Open DPhone ${fw.runningVersion}`],
                ["Model", status?.system?.model],
                ["Running bank", fw.runningBank ? `${fw.runningBank} (${fw.runningBank === "A" ? "k0 + root0" : "k1 + root1"})` : ""],
                ["State", <Badge tone={fw.phase === UpdatePhase.FAILED ? "red" : working ? "blue" : "gray"}>{label(updatePhaseNames, fw.phase, UpdatePhase[fw.phase])}</Badge>],
              ]}
            />
          </Card>

          <Card title="Install firmware">
            {working ? (
              <div className="space-y-3">
                <Bar fraction={overall} label={<>{label(updatePhaseNames, fw.phase, UpdatePhase[fw.phase])}{fw.incomingVersion && ` · ${fw.incomingVersion.replace(/_/g, ".")}`}</>} />
              </div>
            ) : fw.phase === UpdatePhase.COMPLETE ? (
              <Alert tone="green" title={`Firmware ${fw.incomingVersion.replace(/_/g, ".")} is installed and starts at the next reboot.`} />
            ) : sending !== undefined ? (
              <Bar fraction={sending} label="Uploading" />
            ) : up ? (
              <div className="space-y-4">
                <KeyValue
                  rows={[
                    ["Image", up.isStock ? "Stock Digium firmware" : "Open DPhone firmware"],
                    ["Version", up.version.replace(/_/g, ".")],
                    ["For model", up.model],
                    ["Size", bytes(up.size)],
                  ]}
                />
                {!up.modelMatches && <Alert tone="red" title={`This image is for the ${up.model}, not this phone.`} />}
                <div className="flex gap-2">
                  {up.isStock ? (
                    <ConfirmButton label="Install" title="Install stock firmware?" confirmLabel="Install" onConfirm={() => install.mutateAsync({ uploadId: up.uploadId, allowStock: true })} />
                  ) : (
                    <Button variant="primary" disabled={!up.modelMatches} busy={install.isPending} onClick={() => install.mutate({ uploadId: up.uploadId, allowStock: false })}>
                      Install
                    </Button>
                  )}
                  <Button onClick={() => discard.mutate({ uploadId: up.uploadId })}>Discard</Button>
                </div>
              </div>
            ) : (
              <div className="space-y-3">
                {fw.phase === UpdatePhase.FAILED && <Alert tone="red" title="The last install failed.">{fw.error}</Alert>}
                <input ref={file} type="file" accept=".eff" className="hidden" onChange={(e) => e.target.files?.[0] && send(e.target.files[0]).then(() => (e.target.value = ""))} />
                <Button variant="primary" onClick={() => file.current?.click()}>
                  Choose image
                </Button>
              </div>
            )}
          </Card>

          <ProvisionedFirmware />
        </div>
      </div>
    </>
  );
}
