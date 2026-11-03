import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage, team } from "./api";
import type { Member } from "./bindings/Member";

const ROLE: Record<string, string> = { owner: "Owner", editor: "Can edit", viewer: "Can view" };

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  return (parts[0][0] + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
}

/** A steady colour per person, so the same face looks the same everywhere. */
function hue(id: string): number {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  // The golden angle spreads near-identical ids ("u1", "u2") far apart on the colour wheel.
  return Math.round((h * 137.508) % 360);
}

function Avatar({ m }: { m: Member }) {
  return (
    <span className="avatar" style={{ background: `hsl(${hue(m.userId)} 55% 45%)` }} aria-hidden="true">
      {initials(m.name || m.email)}
    </span>
  );
}

/**
 * The people in a team project, in its header: their faces, and on click the list with each
 * one's role, Invite People and (for the owner) Members & Permissions.
 */
export function TeamPeople({ online, onInvite, onManage }: { online: boolean; onInvite: () => void; onManage: () => void }) {
  const [members, setMembers] = useState<Member[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);

  const load = useCallback(() => {
    team.members().then(
      (m) => {
        setMembers(m);
        setProblem(null);
      },
      (e) => setProblem(errorMessage(e)),
    );
  }, []);
  // At once, again when the connection comes back, and whenever the list is opened (someone may
  // have joined).
  useEffect(() => {
    load();
  }, [online, load]);
  useEffect(() => {
    if (open) load();
  }, [open, load]);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => box.current && !box.current.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  const sorted = [...(members ?? [])].sort((a, b) => Number(b.isMe) - Number(a.isMe) || (a.role === "owner" ? -1 : b.role === "owner" ? 1 : 0) || a.name.localeCompare(b.name));
  const iOwn = sorted.some((m) => m.isMe && m.role === "owner");
  const shown = sorted.slice(0, 4);
  const label = members ? `${members.length} ${members.length === 1 ? "person" : "people"} in this project` : "People in this project";

  return (
    <div className="team-people" ref={box}>
      <button className="team-people-button" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="dialog" title={members ? sorted.map((m) => m.name).join(", ") : label} aria-label={label}>
        {members ? (
          <>
            <span className="avatars">
              {shown.map((m) => (
                <Avatar key={m.userId} m={m} />
              ))}
            </span>
            {sorted.length > shown.length && <span className="more">+{sorted.length - shown.length}</span>}
            {sorted.length <= shown.length && <span className="count">{sorted.length}</span>}
          </>
        ) : (
          <span className="count">People</span>
        )}
      </button>
      {open && (
        <div className="team-people-list" role="dialog" aria-label={label}>
          <h2>{label}</h2>
          {problem && !members && <p className="secondary small">{problem}</p>}
          {members && members.length <= 1 && <p className="secondary small">No one else yet. Invite people to work on it with you.</p>}
          <ul>
            {sorted.map((m) => (
              <li key={m.userId}>
                <Avatar m={m} />
                <span className="who">
                  <span>
                    {m.name}
                    {m.isMe && <span className="secondary"> (you)</span>}
                  </span>
                  <span className="secondary small">{m.email}</span>
                </span>
                <span className="role secondary small">{ROLE[m.role] ?? m.role}</span>
              </li>
            ))}
          </ul>
          <div className="team-people-actions">
            <button
              onClick={() => {
                setOpen(false);
                onInvite();
              }}
            >
              Invite People…
            </button>
            {iOwn && (
              <button
                className="link"
                onClick={() => {
                  setOpen(false);
                  onManage();
                }}
              >
                Members & Permissions…
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
