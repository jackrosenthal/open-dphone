import { useMutation } from "@connectrpc/connect-query";
import { useState, type FormEvent } from "react";
import { Badge, Button, Card, ConfirmButton, cx, Empty, Field, Input, Modal, PageHeader, Select, Spinner, Toggle } from "../components/ui";
import { AlarmsService, type Alarm } from "../gen/dphone/v1/alarms_pb";
import { useStreamState } from "../lib/stream";
import { client } from "../lib/transport";
import { ErrorNote } from "./common";

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WEEKDAYS = 0b0111110;
const WEEKENDS = 0b1000001;
const EVERY_DAY = 0b1111111;

/// The days an alarm rings on (bit 0 is Sunday), in a few words.
export function daysLabel(days: number): string {
  if (days === 0) return "Once";
  if (days === EVERY_DAY) return "Every day";
  if (days === WEEKDAYS) return "Weekdays";
  if (days === WEEKENDS) return "Weekends";
  return DAYS.filter((_, i) => days & (1 << i))
    .map((d) => d.slice(0, 3))
    .join(", ");
}

/// A time of day as the browser's locale writes it.
function timeLabel(hour: number, minute: number): string {
  return new Date(2000, 0, 1, hour, minute).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

const pad = (n: number) => String(n).padStart(2, "0");

/// What the form edits: an alarm's fields, with `id` 0 for a new one.
type Draft = Pick<Alarm, "id" | "hour" | "minute" | "days" | "tone" | "enabled">;

function AlarmForm({ draft, tones, onClose }: { draft: Draft; tones: string[]; onClose: () => void }) {
  const [time, setTime] = useState(`${pad(draft.hour)}:${pad(draft.minute)}`);
  const [days, setDays] = useState(draft.days);
  const [tone, setTone] = useState(draft.tone || tones[0] || "");
  const [error, setError] = useState<unknown>();
  const save = useMutation(AlarmsService.method.setAlarm, { onError: setError, onSuccess: onClose });
  const [hour, minute] = time.split(":").map(Number);
  const valid = Number.isInteger(hour) && Number.isInteger(minute);
  function submit(e: FormEvent) {
    e.preventDefault();
    if (valid) save.mutate({ alarm: { id: draft.id, hour, minute, days, tone, enabled: draft.enabled } });
  }
  return (
    <Modal title={draft.id === 0 ? "New Alarm" : "Edit Alarm"} onClose={onClose}>
      <form onSubmit={submit} className="space-y-4">
        <ErrorNote error={error} />
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label="Time">
            <Input type="time" required value={time} onChange={(e) => setTime(e.target.value)} autoFocus />
          </Field>
          <Field label="Ringtone">
            <Select value={tone} onChange={(e) => setTone(e.target.value)}>
              {tones.map((t) => (
                <option key={t}>{t}</option>
              ))}
            </Select>
          </Field>
        </div>
        <fieldset>
          <legend className="mb-1 block text-sm font-medium text-slate-700 dark:text-slate-300">Repeat</legend>
          <div className="flex flex-wrap gap-1.5">
            {DAYS.map((d, i) => {
              const on = (days & (1 << i)) !== 0;
              return (
                <button
                  key={d}
                  type="button"
                  aria-pressed={on}
                  aria-label={d}
                  onClick={() => setDays(days ^ (1 << i))}
                  className={cx(
                    "w-12 rounded-lg px-2 py-1.5 text-sm font-medium ring-1 ring-inset",
                    on
                      ? "bg-brand-600 text-white ring-brand-600"
                      : "bg-white text-slate-700 ring-slate-300 hover:bg-slate-50 dark:bg-slate-800 dark:text-slate-300 dark:ring-slate-600 dark:hover:bg-slate-700",
                  )}
                >
                  {d.slice(0, 3)}
                </button>
              );
            })}
          </div>
          <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{days === 0 ? "Rings once" : daysLabel(days)}</p>
        </fieldset>
        <div className="flex justify-end gap-2">
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" busy={save.isPending} disabled={!valid}>
            Save
          </Button>
        </div>
      </form>
    </Modal>
  );
}

export function Alarms() {
  const [state] = useStreamState((signal) => client(AlarmsService).watchAlarms({}, { signal }));
  const [error, setError] = useState<unknown>();
  const [editing, setEditing] = useState<Draft>();
  const opts = { onError: setError, onSuccess: () => setError(undefined) };
  const set = useMutation(AlarmsService.method.setAlarm, opts);
  const remove = useMutation(AlarmsService.method.removeAlarm, opts);
  const snooze = useMutation(AlarmsService.method.snoozeAlarm, opts);
  const dismiss = useMutation(AlarmsService.method.dismissAlarm, opts);

  const snap = state?.snapshot;
  if (!snap) {
    return (
      <>
        <PageHeader title="Alarms" />
        <Spinner />
      </>
    );
  }
  const full = snap.alarms.length >= snap.maxAlarms;
  const fresh: Draft = { id: 0, hour: 8, minute: 0, days: 0, tone: snap.tones[0] ?? "", enabled: true };

  return (
    <>
      <PageHeader
        title="Alarms"
        actions={
          <Button variant="primary" disabled={full} title={full ? `At most ${snap.maxAlarms} alarms` : undefined} onClick={() => setEditing(fresh)}>
            New Alarm
          </Button>
        }
      />
      <div className="space-y-5">
        <ErrorNote error={error} />
        <Card>
          {snap.alarms.length === 0 ? (
            <Empty>No alarms</Empty>
          ) : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {snap.alarms.map((a) => {
                const ringing = snap.ringing === a.id;
                const snoozed = snap.snoozed?.id === a.id;
                return (
                  <li key={a.id} className="flex flex-wrap items-center justify-between gap-3 py-3 first:pt-0 last:pb-0">
                    <div className={cx("min-w-0", !a.enabled && !ringing && !snoozed && "opacity-50")}>
                      <p className="text-xl font-semibold tabular-nums">{timeLabel(a.hour, a.minute)}</p>
                      <p className="truncate text-sm text-slate-500 dark:text-slate-400">
                        {daysLabel(a.days)} · {a.tone}
                      </p>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      {ringing && (
                        <>
                          <Badge tone="amber">Ringing</Badge>
                          <Button size="sm" onClick={() => snooze.mutate({})}>
                            Snooze
                          </Button>
                        </>
                      )}
                      {snoozed && <Badge tone="blue">Snoozed</Badge>}
                      {(ringing || snoozed) && (
                        <Button size="sm" onClick={() => dismiss.mutate({})}>
                          Dismiss
                        </Button>
                      )}
                      <Toggle label={a.enabled ? "On" : "Off"} checked={a.enabled} disabled={set.isPending} onChange={(enabled) => set.mutate({ alarm: { ...a, enabled } })} />
                      <Button size="sm" onClick={() => setEditing(a)}>
                        Edit
                      </Button>
                      <ConfirmButton
                        size="sm"
                        variant="secondary"
                        label="Delete"
                        title={`Delete the ${timeLabel(a.hour, a.minute)} alarm?`}
                        confirmLabel="Delete"
                        onConfirm={() => remove.mutateAsync({ id: a.id })}
                      />
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </Card>
      </div>
      {editing && <AlarmForm key={editing.id} draft={editing} tones={snap.tones} onClose={() => setEditing(undefined)} />}
    </>
  );
}
