import { create } from "@bufbuild/protobuf";
import { useMutation, useQuery } from "@connectrpc/connect-query";
import { useRef, useState } from "react";
import { Icons } from "../components/icons";
import { Badge, Button, Card, ConfirmButton, Empty, Field, Input, Modal, PageHeader, Select, Spinner, td, th } from "../components/ui";
import { ContactGroupSchema, ContactNumberSchema, ContactSchema, ContactsService, type Contact, type ContactGroup } from "../gen/dphone/v1/contacts_pb";
import { CallsService } from "../gen/dphone/v1/calls_pb";
import { client, errorMessage } from "../lib/transport";
import { ErrorNote } from "./common";

function ContactEditor({ contact, onSave, onClose }: { contact: Contact; onSave: (c: Contact) => void; onClose: () => void }) {
  const [c, setC] = useState(contact);
  const setNumber = (i: number, patch: Partial<Contact["numbers"][number]>) => setC({ ...c, numbers: c.numbers.map((n, j) => (j === i ? { ...n, ...patch } : patch.primary ? { ...n, primary: false } : n)) });
  return (
    <Modal
      title={contact.id ? "Edit contact" : "New contact"}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={() => onSave({ ...c, numbers: c.numbers.filter((n) => n.dial.trim()) })}>
            Done
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label="First name">
            <Input value={c.firstName} onChange={(e) => setC({ ...c, firstName: e.target.value })} autoFocus />
          </Field>
          <Field label="Last name">
            <Input value={c.lastName} onChange={(e) => setC({ ...c, lastName: e.target.value })} />
          </Field>
          <Field label="Organization">
            <Input value={c.organization} onChange={(e) => setC({ ...c, organization: e.target.value })} />
          </Field>
          <Field label="Presence extension" hint="Extension whose status this contact shows.">
            <Input value={c.subscribeTo} onChange={(e) => setC({ ...c, subscribeTo: e.target.value })} />
          </Field>
        </div>
        <div>
          <p className="mb-2 text-sm font-medium text-slate-700 dark:text-slate-300">Numbers</p>
          <div className="space-y-2">
            {c.numbers.map((n, i) => (
              <div key={i} className="flex items-center gap-2">
                <Input className="flex-1" placeholder="Number" value={n.dial} onChange={(e) => setNumber(i, { dial: e.target.value })} aria-label="Number" />
                <Input className="w-32" placeholder="Label" value={n.label} onChange={(e) => setNumber(i, { label: e.target.value })} aria-label="Label" />
                <label className="flex items-center gap-1 text-xs text-slate-500">
                  <input type="radio" name="primary" checked={n.primary} onChange={() => setNumber(i, { primary: true })} />
                  Primary
                </label>
                <button type="button" aria-label="Remove number" className="rounded p-1 text-slate-400 hover:text-red-600" onClick={() => setC({ ...c, numbers: c.numbers.filter((_, j) => j !== i) })}>
                  <Icons.trash className="size-4" />
                </button>
              </div>
            ))}
          </div>
          <Button size="sm" className="mt-2" onClick={() => setC({ ...c, numbers: [...c.numbers, create(ContactNumberSchema, { primary: c.numbers.length === 0 })] })}>
            <Icons.plus className="size-3.5" /> Add number
          </Button>
        </div>
      </div>
    </Modal>
  );
}

function ListEditor({ name }: { name: string }) {
  const list = useQuery(ContactsService.method.getContactList, { name });
  const [groups, setGroups] = useState<ContactGroup[]>();
  const [editing, setEditing] = useState<{ group: number; index: number; contact: Contact }>();
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<unknown>();
  const put = useMutation(ContactsService.method.putContactList, {
    onSuccess: () => {
      setGroups(undefined);
      setError(undefined);
      list.refetch();
    },
    onError: setError,
  });
  const dial = useMutation(CallsService.method.dial, { onError: setError });

  if (list.error) return <ErrorNote error={list.error} />;
  if (!list.data) return <Spinner />;
  const current = groups ?? list.data.groups;
  const dirty = groups !== undefined;
  const q = filter.trim().toLowerCase();

  const update = (gi: number, contacts: Contact[]) => setGroups(current.map((g, i) => (i === gi ? { ...g, contacts } : g)));

  return (
    <div className="space-y-4">
      <ErrorNote error={error} />
      <div className="flex flex-wrap items-center gap-2">
        <Input className="max-w-xs" placeholder="Search contacts" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Search contacts" />
        <div className="flex-1" />
        {dirty && <Badge tone="amber">Unsaved changes</Badge>}
        {dirty && <Button onClick={() => setGroups(undefined)}>Discard</Button>}
        <Button variant="primary" disabled={!dirty} busy={put.isPending} onClick={() => put.mutate({ name, groups: current })}>
          Save list
        </Button>
      </div>
      {current.length === 0 && (
        <Button onClick={() => setGroups([create(ContactGroupSchema, { id: "local", name: "Contacts" })])}>Add a group</Button>
      )}
      {current.map((g, gi) => {
        const rows = g.contacts.map((c, index) => ({ c, index })).filter(({ c }) => !q || `${c.firstName} ${c.lastName} ${c.organization} ${c.numbers.map((n) => n.dial).join(" ")}`.toLowerCase().includes(q));
        return (
          <div key={gi}>
            <div className="mb-2 flex items-center justify-between">
              <h3 className="text-sm font-semibold">
                {g.name || g.id || "Group"} <span className="font-normal text-slate-400">{g.contacts.length}</span>
              </h3>
              <Button size="sm" onClick={() => setEditing({ group: gi, index: -1, contact: create(ContactSchema, { contactType: "sip", numbers: [create(ContactNumberSchema, { primary: true })] }) })}>
                <Icons.plus className="size-3.5" /> Add contact
              </Button>
            </div>
            {rows.length === 0 ? (
              <Empty>{q ? "No matches." : "This group is empty."}</Empty>
            ) : (
              <div className="overflow-x-auto">
                <table className="w-full">
                  <thead>
                    <tr>
                      <th className={th}>Name</th>
                      <th className={th}>Organization</th>
                      <th className={th}>Numbers</th>
                      <th className={th} />
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                    {rows.map(({ c, index }) => (
                      <tr key={index}>
                        <td className={`${td} font-medium`}>{`${c.firstName} ${c.lastName}`.trim() || "—"}</td>
                        <td className={td}>{c.organization}</td>
                        <td className={td}>
                          {c.numbers.map((n, i) => (
                            <button key={i} type="button" title={`Call ${n.dial}`} className="mr-3 text-brand-700 hover:underline dark:text-brand-300" onClick={() => dial.mutate({ target: n.dial })}>
                              {n.dial}
                              {n.label && <span className="ml-1 text-xs text-slate-400">{n.label}</span>}
                            </button>
                          ))}
                        </td>
                        <td className={`${td} whitespace-nowrap text-right`}>
                          <Button size="sm" onClick={() => setEditing({ group: gi, index, contact: c })}>
                            Edit
                          </Button>{" "}
                          <Button size="sm" variant="ghost" aria-label="Delete contact" onClick={() => update(gi, g.contacts.filter((_, j) => j !== index))}>
                            <Icons.trash className="size-4" />
                          </Button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      })}
      {editing && (
        <ContactEditor
          contact={editing.contact}
          onClose={() => setEditing(undefined)}
          onSave={(c) => {
            const g = current[editing.group];
            update(editing.group, editing.index < 0 ? [...g.contacts, c] : g.contacts.map((x, j) => (j === editing.index ? c : x)));
            setEditing(undefined);
          }}
        />
      )}
    </div>
  );
}

export function Contacts() {
  const lists = useQuery(ContactsService.method.listContactLists, {});
  const [selected, setSelected] = useState<string>();
  const [error, setError] = useState("");
  const file = useRef<HTMLInputElement>(null);
  const remove = useMutation(ContactsService.method.deleteContactList, { onSuccess: () => { setSelected(undefined); lists.refetch(); } });

  if (lists.error) return <ErrorNote error={lists.error} />;
  if (!lists.data) return <Spinner />;
  const names = lists.data.lists;
  const name = selected && names.some((l) => l.name === selected) ? selected : names[0]?.name;

  async function importFile(f: File) {
    setError("");
    try {
      const xml = await f.text();
      const target = f.name.replace(/[^A-Za-z0-9_.-]/g, "_") || "imported.xml";
      await client(ContactsService).importContactList({ name: target, xml });
      await lists.refetch();
      setSelected(target);
    } catch (e) {
      setError(errorMessage(e));
    }
  }
  async function exportList() {
    if (!name) return;
    const r = await client(ContactsService).exportContactList({ name });
    const url = URL.createObjectURL(new Blob([r.xml], { type: "application/xml" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = name.endsWith(".xml") ? name : `${name}.xml`;
    a.click();
    URL.revokeObjectURL(url);
  }
  async function newList() {
    setError("");
    try {
      let n = "local.xml";
      for (let i = 2; names.some((l) => l.name === n); i++) n = `local-${i}.xml`;
      await client(ContactsService).putContactList({ name: n, groups: [create(ContactGroupSchema, { id: "local", name: "Contacts" })] });
      await lists.refetch();
      setSelected(n);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <>
      <PageHeader
        title="Contacts"
        description="Contact lists stored on the phone, in the stock XML format."
        actions={
          <>
            <input ref={file} type="file" accept=".xml,application/xml,text/xml" className="hidden" onChange={(e) => e.target.files?.[0] && importFile(e.target.files[0]).then(() => (e.target.value = ""))} />
            <Button onClick={() => file.current?.click()}>Import XML</Button>
            <Button onClick={exportList} disabled={!name}>
              Export XML
            </Button>
            <Button variant="primary" onClick={newList}>
              New list
            </Button>
          </>
        }
      />
      <div className="space-y-5">
        {error && <ErrorNote error={error} />}
        {names.length === 0 ? (
          <Card>
            <Empty>No contact lists yet. Create one, or import a stock contacts XML file.</Empty>
          </Card>
        ) : (
          <Card
            title={
              <span className="flex items-center gap-3">
                <Select value={name} onChange={(e) => setSelected(e.target.value)} aria-label="Contact list" className="!w-auto">
                  {names.map((l) => (
                    <option key={l.name} value={l.name}>
                      {l.name} ({l.error ? "unreadable" : `${l.contacts} contacts`})
                    </option>
                  ))}
                </Select>
              </span>
            }
            actions={name && <ConfirmButton size="sm" variant="secondary" label="Delete list" title={`Delete ${name}?`} message="The whole list is removed from the phone." confirmLabel="Delete" onConfirm={() => remove.mutateAsync({ name })} />}
          >
            {name && <ListEditor key={name} name={name} />}
          </Card>
        )}
      </div>
    </>
  );
}
