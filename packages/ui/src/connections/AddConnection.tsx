/**
 * The dialog that adds a connection or changes one: a Rex server, a machine reached over
 * SSH, or which Linux distributions of this machine are listed.
 *
 * An address is read as it is typed, and what it means is said under it. Testing is
 * optional: it goes through what connecting would, step by step, and says where it
 * stopped and what can be done about it. Enter in a field adds, Escape puts the dialog
 * away, Tab stays in it.
 *
 * It is the same dialog in a window of terminals and in the settings window, so what it
 * works with is handed to it.
 */

import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import type { CoreClient } from "../core/client.ts";
import type { ConfigPlatform, Profile, RexHost } from "../core/types.ts";
import { Glyph } from "../icons/Glyph.tsx";
import { KindIcon } from "../connect/icons.tsx";
import { sentence } from "../shell/notices.ts";
import { CommandError } from "../transport/types.ts";
import {
  addressWords,
  defaultName,
  distroChanges,
  distroRows,
  distroWords,
  EMPTY_SSH,
  failedStep,
  knownHosts,
  labelTaken,
  nameOfHost,
  rexSteps,
  sshForm,
  sshHost,
  sshSteps,
  testBegun,
  testFailed,
  testFailure,
  testGoesOn,
  testWords,
  triedWords,
  type AddressRead,
  type ConnectionKind,
  type ConnectionTest,
  type DistroRow,
  type SshForm,
} from "./model.ts";
import type { ConnectionAsked } from "./store.ts";

/** What was added or changed, for whoever showed the dialog. */
export type ConnectionDone =
  | { readonly kind: "rex"; readonly label: string }
  | { readonly kind: "ssh"; readonly profile: string }
  | { readonly kind: "wsl" };

/** What the dialog works with. */
export interface ConnectionServices {
  readonly client: CoreClient;
  readonly hosts: readonly RexHost[];
  readonly profiles: readonly Profile[];
  readonly platform: ConfigPlatform;
}

const KINDS: readonly {
  readonly kind: ConnectionKind;
  readonly label: string;
  readonly line: string;
}[] = [
  { kind: "rex", label: "Rex", line: "Sessions keep running when you disconnect" },
  { kind: "ssh", label: "SSH", line: "A shell on any machine you can SSH to" },
  { kind: "wsl", label: "WSL", line: "Linux distros on this PC" },
];

function said(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function Field({
  id,
  label,
  problem,
  children,
}: {
  id: string;
  label: string;
  problem?: string | null;
  children: ReactNode;
}) {
  return (
    <div className="connection-field">
      <label htmlFor={id}>{label}</label>
      {children}
      {problem != null && (
        <span className="connection-problem" id={`${id}-problem`}>
          {problem}
        </span>
      )}
    </div>
  );
}

function StepMark({ state }: { state: "wait" | "run" | "ok" | "bad" }) {
  switch (state) {
    case "ok":
      return <Glyph name="check" size={13} weight={2.6} />;
    case "bad":
      return <Glyph name="x" size={13} weight={2.6} />;
    case "run":
      return <span className="connection-spin" />;
    default:
      return <span className="connection-dot" />;
  }
}

export function AddConnection({
  asked,
  services,
  close,
  done,
}: {
  asked: ConnectionAsked;
  services: ConnectionServices;
  close: () => void;
  /** Called once it is written, after the dialog was put away. */
  done: (what: ConnectionDone) => void;
}) {
  const { client, hosts, profiles, platform } = services;
  const edit = asked.edit;
  const box = useRef<HTMLDivElement>(null);
  const [kind, setKind] = useState<ConnectionKind>(asked.kind);
  const [address, setAddress] = useState(edit?.kind === "rex" ? edit.address : "");
  // How the address that was asked about last came out, and which that was.
  const [answer, setAnswer] = useState<{ typed: string; read: AddressRead } | null>(null);
  const [ssh, setSsh] = useState<SshForm>(() =>
    edit?.kind === "ssh" ? sshForm(edit.profile) : EMPTY_SSH,
  );
  const [name, setName] = useState(
    edit === null ? "" : edit.kind === "rex" ? edit.label : edit.profile.name,
  );
  const [distros, setDistros] = useState<readonly DistroRow[]>(() => distroRows(profiles));
  const [test, setTest] = useState<ConnectionTest | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);
  // Counts what was typed and the tests begun: an answer to an earlier one is passed over.
  const asking = useRef(0);
  const testing = useRef(0);

  const was = edit?.kind === "rex" ? edit.label : null;
  const before = distroRows(profiles);

  // The profiles may be known only after the dialog is up.
  const found = before.map((row) => row.id).join(" ");
  useEffect(() => {
    setDistros((held) =>
      distroRows(profiles).map((row) => held.find((one) => one.id === row.id) ?? row),
    );
    // Only which there are matters: what is ticked stays while a row's words change.
  }, [found]);

  useEffect(() => {
    const turn = ++asking.current;
    if (address.trim() === "") {
      setAnswer(null);
      return;
    }
    const take = (read: AddressRead) => {
      if (asking.current === turn) setAnswer({ typed: address, read });
    };
    client.rexRead(address, was).then(
      (found) => take({ ok: true, address: found }),
      (error: unknown) => {
        const code = error instanceof CommandError ? error.code : "bad_request";
        take({ ok: false, code, message: said(error) });
      },
    );
  }, [client, address, was]);

  useEffect(() => {
    box.current?.querySelector<HTMLElement>("[data-first]")?.focus();
  }, [kind]);

  // A test of what is no longer typed is forgotten.
  const forget = () => {
    testing.current++;
    setTest(null);
    setProblem(null);
  };
  const pick = (next: ConnectionKind) => {
    if (next === kind) return;
    forget();
    setKind(next);
  };
  const type = (value: string) => {
    forget();
    setAddress(value);
  };
  const typeSsh = (patch: Partial<SshForm>) => {
    forget();
    setSsh((held) => ({ ...held, ...patch }));
  };

  const read = answer?.read ?? null;
  const machine = sshHost(ssh);
  const byDefault = kind === "rex" ? defaultName(read) : kind === "ssh" ? nameOfHost(ssh.host) : "";
  const label = name.trim() === "" ? byDefault : name.trim();
  const clash = kind === "rex" && label !== "" ? labelTaken(label, hosts, was) : null;
  const words = addressWords(address, read);
  // What was typed since is not known yet to be an address.
  const usable = answer?.typed === address && read?.ok === true && read.address.taken === null;
  const changes = distroChanges(before, distros);
  const ready =
    !writing &&
    (kind === "rex"
      ? usable && label !== "" && clash === null && test?.failure?.final !== true
      : kind === "ssh"
        ? typeof machine !== "string" && label !== ""
        : changes.length > 0);

  const runRex = (agreed: boolean) => {
    if (read?.ok !== true) return;
    const target = read.address;
    const turn = ++testing.current;
    const current = () => testing.current === turn;
    let going = testBegun(rexSteps(target));
    const show = (next: ConnectionTest) => {
      going = next;
      setTest(next);
    };
    show(going);
    const stop = (index: number, code: string, message: string) => {
      if (current()) show(testFailed(going, index, testFailure(code, message)));
    };
    client.rexReach(address).then(
      (reached) => {
        if (!current()) return;
        if (reached.ms === null && reached.relayed !== true) {
          stop(0, "unreachable", reached.note ?? "the machine did not answer");
          return;
        }
        const how = reached.ms === null ? (reached.note ?? "") : `${reached.ms} ms`;
        const near = target.kind === "local" || target.kind === "pipe" || target.kind === "unix";
        show(testGoesOn(going, 1, [near ? "" : how]));
        client.rexTry(address, agreed).then(
          (tried) => {
            if (!current()) return;
            const steps = going.steps.length;
            const last = [...Array<string>(steps - 1), triedWords(tried)];
            show(testGoesOn(going, steps, last));
          },
          (error: unknown) => {
            const code = error instanceof CommandError ? error.code : "internal";
            stop(failedStep(target, code), code, said(error));
          },
        );
      },
      (error: unknown) => stop(0, "unreachable", said(error)),
    );
  };
  const runSsh = () => {
    if (typeof machine === "string") return;
    const turn = ++testing.current;
    const current = () => testing.current === turn;
    let going = testBegun(sshSteps(machine));
    const show = (next: ConnectionTest) => {
      going = next;
      setTest(next);
    };
    show(going);
    const stop = (index: number, code: string, message: string) => {
      if (current()) show(testFailed(going, index, testFailure(code, message)));
    };
    client.sshReach(machine).then(
      (reached) => {
        if (!current()) return;
        if (reached.ms === null && reached.relayed !== true) {
          stop(0, "unreachable", reached.note ?? "the machine did not answer");
          return;
        }
        show(
          testGoesOn(going, 1, [reached.ms === null ? (reached.note ?? "") : `${reached.ms} ms`]),
        );
        client.sshTry(machine).then(
          () => {
            if (current()) {
              show(
                testGoesOn(going, 3, [going.steps[0]?.detail ?? "", ssh.withKey ? ssh.key : ""]),
              );
            }
          },
          (error: unknown) =>
            stop(1, error instanceof CommandError ? error.code : "internal", said(error)),
        );
      },
      (error: unknown) => stop(0, "unreachable", said(error)),
    );
  };
  const testable = test?.state !== "run" && (kind === "rex" ? usable : typeof machine !== "string");
  const runTest = () => {
    if (!testable) return;
    if (kind === "rex") runRex(false);
    else runSsh();
  };

  const write = () => {
    if (!ready) return;
    setWriting(true);
    setProblem(null);
    const writes: Promise<ConnectionDone> =
      kind === "rex"
        ? client.rexHostPut(was, label, address.trim()).then(() => ({ kind, label }))
        : kind === "ssh" && typeof machine !== "string"
          ? client
              .sshProfilePut(edit?.kind === "ssh" ? edit.profile.id : null, label, machine)
              .then((profile) => ({ kind, profile }))
          : client.profilesListed(changes).then(() => ({ kind: "wsl" }));
    writes.then(
      (what) => {
        close();
        done(what);
      },
      (error: unknown) => {
        setWriting(false);
        setProblem(sentence(said(error)));
      },
    );
  };
  /** Goes on with plain SSH on the machine a Rex server was not found on. */
  const useSsh = () => {
    if (read?.ok !== true) return;
    const target = read.address;
    forget();
    setSsh({
      ...EMPTY_SSH,
      host: target.host ?? "",
      user: target.user ?? "",
      port: String(target.port ?? 22),
    });
    setKind("ssh");
  };

  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    // No key of the dialog's is one for a terminal or the window behind it.
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    const target = event.target as HTMLElement;
    if (event.key === "Enter" && target.matches("input:not([type=checkbox])")) {
      event.preventDefault();
      write();
      return;
    }
    if (event.key !== "Tab" || !box.current) return;
    // The keyboard stays in the dialog.
    const stops = [...box.current.querySelectorAll<HTMLElement>("button, input")].filter(
      (element) => !element.matches(":disabled, [tabindex='-1']"),
    );
    const first = stops[0];
    const last = stops.at(-1);
    if (event.shiftKey ? target === first : target === last) {
      event.preventDefault();
      (event.shiftKey ? last : first)?.focus();
    }
  };
  // A distribution of this machine is a thing of Windows.
  const kinds = KINDS.filter((one) => one.kind !== "wsl" || platform === "windows");
  const onKindKey = (event: KeyboardEvent<HTMLElement>) => {
    const by = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (by === 0 || edit !== null) return;
    event.preventDefault();
    const at = kinds.findIndex((one) => one.kind === kind);
    const next = kinds[(at + by + kinds.length) % kinds.length];
    if (next) pick(next.kind);
  };

  const offered = kind === "rex" && edit === null ? knownHosts(profiles, hosts) : [];
  const title =
    edit === null
      ? "Add connection"
      : `Edit ${edit.kind === "rex" ? edit.label : edit.profile.name}`;
  const status = problem ?? (kind === "wsl" ? distroWords(distros) : testWords(test));
  const addLabel = edit !== null || kind === "wsl" ? "Save" : "Add connection";
  const failure = test?.failure ?? null;

  return (
    <>
      <div className="connection-scrim" aria-hidden="true" onPointerDown={close} />
      <div
        ref={box}
        className="connection"
        role="dialog"
        aria-modal="true"
        aria-labelledby="connection-title"
        data-kind={kind}
        data-animate={!asked.keyboard || undefined}
        onKeyDown={onKey}
      >
        <div className="connection-head">
          <h2 id="connection-title">{title}</h2>
          <span className="open-folder-key">Esc</span>
        </div>
        <div
          className="connection-kinds"
          role="radiogroup"
          aria-label="Connection type"
          onKeyDown={onKindKey}
        >
          {kinds.map((one) => (
            <button
              key={one.kind}
              type="button"
              className="connection-kind"
              role="radio"
              data-kind={one.kind}
              aria-checked={one.kind === kind}
              tabIndex={one.kind === kind ? 0 : -1}
              disabled={edit !== null}
              onClick={() => pick(one.kind)}
            >
              <b>
                {one.kind === "rex" ? (
                  <Glyph name="server" size={15} weight={1.8} />
                ) : (
                  <KindIcon kind={one.kind} size={15} />
                )}
                {one.label}
                {one.kind === "rex" && <span className="connection-best">Recommended</span>}
              </b>
              <small>{one.line}</small>
            </button>
          ))}
        </div>
        <div className="connection-body">
          {kind === "rex" && (
            <>
              <Field id="connection-address" label="Address">
                <input
                  id="connection-address"
                  className="connection-input"
                  data-first
                  data-mono
                  value={address}
                  placeholder="ssh://alice@build.example.com"
                  autoComplete="off"
                  spellCheck={false}
                  aria-describedby="connection-read"
                  onChange={(event) => type(event.target.value)}
                />
              </Field>
              <div
                id="connection-read"
                className="connection-read"
                data-tone={words.tone}
                role="status"
              >
                <span>
                  <Glyph name={words.icon} size={14} />
                </span>
                <span>
                  <b>{words.head}</b>
                  {words.text}
                </span>
              </div>
            </>
          )}
          {kind === "ssh" && (
            <div className="connection-three">
              <Field id="connection-host" label="Host">
                <input
                  id="connection-host"
                  className="connection-input"
                  data-first
                  data-mono
                  value={ssh.host}
                  placeholder="build.example.com"
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(event) => typeSsh({ host: event.target.value })}
                />
              </Field>
              <Field id="connection-user" label="User">
                <input
                  id="connection-user"
                  className="connection-input"
                  data-mono
                  value={ssh.user}
                  placeholder="from ssh config"
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(event) => typeSsh({ user: event.target.value })}
                />
              </Field>
              <Field id="connection-port" label="Port">
                <input
                  id="connection-port"
                  className="connection-input"
                  data-mono
                  value={ssh.port}
                  inputMode="numeric"
                  autoComplete="off"
                  onChange={(event) => typeSsh({ port: event.target.value.replace(/\D/g, "") })}
                />
              </Field>
            </div>
          )}
          {offered.length > 0 && (
            <div className="connection-field">
              <span className="connection-label">From your ~/.ssh/config</span>
              <div className="connection-chips">
                {offered.map((host) => (
                  <button
                    key={host.name}
                    type="button"
                    className="connection-chip"
                    disabled={host.added}
                    title={host.address}
                    onClick={() => {
                      type(host.address);
                      if (name.trim() === "") setName(host.name);
                    }}
                  >
                    {host.name}
                    {host.added && <small>added</small>}
                  </button>
                ))}
              </div>
            </div>
          )}
          {kind === "ssh" && (
            <div className="connection-field">
              <span className="connection-label">Sign in with</span>
              <div className="connection-auth">
                <div className="connection-segmented" role="group" aria-label="Sign in with">
                  <button
                    type="button"
                    aria-pressed={!ssh.withKey}
                    onClick={() => typeSsh({ withKey: false })}
                  >
                    ssh-agent and config
                  </button>
                  <button
                    type="button"
                    aria-pressed={ssh.withKey}
                    onClick={() => typeSsh({ withKey: true })}
                  >
                    A key file
                  </button>
                </div>
                {ssh.withKey && (
                  <input
                    className="connection-input"
                    data-mono
                    aria-label="Key file"
                    value={ssh.key}
                    autoComplete="off"
                    spellCheck={false}
                    onChange={(event) => typeSsh({ key: event.target.value })}
                  />
                )}
              </div>
            </div>
          )}
          {kind !== "wsl" && (
            <Field id="connection-name" label="Name" problem={clash}>
              <input
                id="connection-name"
                className="connection-input"
                value={name}
                placeholder={byDefault === "" ? "build" : byDefault}
                autoComplete="off"
                spellCheck={false}
                aria-invalid={clash !== null}
                aria-describedby={clash === null ? undefined : "connection-name-problem"}
                onChange={(event) => setName(event.target.value)}
              />
            </Field>
          )}
          {kind === "ssh" && (
            <p className="connection-note">
              Plain SSH sessions end when the connection drops. Add the machine as a Rex host to
              keep them running.
            </p>
          )}
          {kind === "wsl" && (
            <>
              <div className="connection-field">
                <span className="connection-label">Distros on this PC</span>
                <div className="connection-distros">
                  {distros.map((row, index) => {
                    const set = (patch: Partial<DistroRow>) =>
                      setDistros((held) =>
                        held.map((one) => (one.id === row.id ? { ...one, ...patch } : one)),
                      );
                    const line = before.find((one) => one.id === row.id)?.line ?? row.line;
                    return (
                      <div key={row.id} className="connection-distro">
                        <span className="connection-distro-icon">
                          <KindIcon kind="wsl" size={15} />
                        </span>
                        <span className="connection-distro-name">
                          {row.name}
                          <small>{line}</small>
                        </span>
                        <input
                          className="connection-input"
                          data-mono
                          aria-label={`Start ${row.name} in`}
                          title="Where its terminals start"
                          placeholder="Start in ~"
                          value={row.cwd}
                          disabled={!row.listed}
                          autoComplete="off"
                          spellCheck={false}
                          onChange={(event) => set({ cwd: event.target.value })}
                        />
                        <input
                          type="checkbox"
                          data-first={index === 0 || undefined}
                          aria-label={`List ${row.name}`}
                          aria-checked={row.listed}
                          checked={row.listed}
                          onChange={(event) => set({ listed: event.target.checked })}
                        />
                      </div>
                    );
                  })}
                  {distros.length === 0 && (
                    <p className="connection-note">No WSL distro was found on this PC.</p>
                  )}
                </div>
              </div>
              <p className="connection-note">
                Found with <code>wsl --list</code>. Stopped distros start when you open a session.
              </p>
            </>
          )}
          {kind !== "wsl" && test !== null && (
            <div className="connection-steps" role="status" aria-live="polite">
              {test.steps.map((step) => (
                <div key={step.text} className="connection-step" data-state={step.state}>
                  <i>
                    <StepMark state={step.state} />
                  </i>
                  {step.text}
                  <em>{step.detail}</em>
                </div>
              ))}
              {failure !== null && (
                <div className="connection-fix">
                  <span>
                    {failure.code === "not_installed" && read?.ok === true ? (
                      <>
                        Install Rex on <b>{read.address.host}</b> and test again, or connect with
                        plain SSH for now.
                      </>
                    ) : (
                      failure.message
                    )}
                  </span>
                  {failure.code === "not_installed" && (
                    <button type="button" className="connection-button" onClick={useSsh}>
                      Use SSH instead
                    </button>
                  )}
                  {failure.code === "confirmation_required" && (
                    <button
                      type="button"
                      className="connection-button"
                      onClick={() => runRex(true)}
                    >
                      Test anyway
                    </button>
                  )}
                </div>
              )}
            </div>
          )}
        </div>
        <div className="connection-foot">
          <span
            className="connection-status"
            data-problem={problem !== null || undefined}
            role="status"
            title={status}
          >
            {status}
          </span>
          {kind !== "wsl" && (
            <button
              type="button"
              className="connection-button"
              disabled={!testable}
              onClick={runTest}
            >
              Test
            </button>
          )}
          <button type="button" className="connection-button" onClick={close}>
            Cancel
          </button>
          <button
            type="button"
            className="connection-button"
            data-primary
            disabled={!ready}
            onClick={write}
          >
            {addLabel}
          </button>
        </div>
      </div>
    </>
  );
}
