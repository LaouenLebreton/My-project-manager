import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { chat, errorMessage, type Snapshot } from "./api";
import type { ChatChannel } from "./bindings/ChatChannel";
import type { ChatPost } from "./bindings/ChatPost";
import type { ChatTeam } from "./bindings/ChatTeam";
import type { ChatThread } from "./bindings/ChatThread";

type ChatEvent = { kind: "post"; teamId: string; post: ChatPost } | { kind: "channel"; teamId: string; channel: ChatChannel };

/** "10:42" today, "Yesterday 16:05", "5 Oct 09:12". */
function when(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const now = new Date();
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOf(now) - startOf(d)) / 86_400_000);
  if (days === 0) return time;
  if (days === 1) return `Yesterday ${time}`;
  return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${time}`;
}

/** Text with @mentions highlighted and #12 turned into a link to task 12 of the open project. */
function Body({ text, onTask, rowCount }: { text: string; onTask: ((row: number) => void) | null; rowCount: number }) {
  const parts = text.split(/(@[\p{L}\p{N}._-]+|#\d+)/u);
  return (
    <div className="chat-body">
      {parts.map((part, i) => {
        if (part.startsWith("@") && part.length > 1)
          return (
            <span key={i} className="mention">
              {part}
            </span>
          );
        const task = /^#(\d+)$/.exec(part);
        if (task && onTask && Number(task[1]) >= 1 && Number(task[1]) <= rowCount)
          return (
            <button key={i} className="link task-ref" onClick={() => onTask(Number(task[1]))} title={`Show task ${task[1]}`}>
              {part}
            </button>
          );
        return <span key={i}>{part}</span>;
      })}
    </div>
  );
}

function Author({ post, mine }: { post: ChatPost; mine: boolean }) {
  return (
    <div className="chat-meta">
      <span className="avatar" aria-hidden>
        {post.authorName.slice(0, 1).toUpperCase()}
      </span>
      <strong>{mine ? `${post.authorName} (you)` : post.authorName}</strong>
      <span className="secondary small">{when(post.at)}</span>
    </div>
  );
}

/** Writes a post or a reply: Return sends, Shift-Return makes a new line. */
function Composer({ placeholder, onSend, autoFocus }: { placeholder: string; onSend: (body: string) => Promise<void>; autoFocus?: boolean }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const send = async () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    try {
      await onSend(text);
      setText("");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="composer">
      <textarea
        rows={Math.min(6, Math.max(1, text.split("\n").length))}
        value={text}
        placeholder={placeholder}
        aria-label={placeholder}
        autoFocus={autoFocus}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void send();
          }
        }}
      />
      <button className="primary" onClick={() => void send()} disabled={!text.trim() || busy}>
        Send
      </button>
    </div>
  );
}

/**
 * Chat, as in Microsoft Teams: teams and their channels on the left; in a channel, conversations
 * (a post and its replies) with the newest at the bottom.
 */
export function ChatView({ snapshot, onShowTask }: { snapshot: Snapshot; onShowTask: (row: number) => void }) {
  const [teams, setTeams] = useState<ChatTeam[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [threads, setThreads] = useState<ChatThread[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [me, setMe] = useState<string | null>(null);
  const [replying, setReplying] = useState<string | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [newChannel, setNewChannel] = useState("");
  const bottom = useRef<HTMLDivElement>(null);
  const selectedRef = useRef<string | null>(null);
  selectedRef.current = selected;

  const loadTeams = useCallback(() => {
    chat.teams().then(
      (t) => {
        setTeams(t);
        setProblem(null);
        setSelected((s) => {
          if (s && t.some((x) => x.channels.some((c) => c.id === s))) return s;
          const all = t.flatMap((x) => x.channels);
          return (all.find((c) => c.name === "General") ?? all[0])?.id ?? null;
        });
      },
      (e) => setProblem(errorMessage(e)),
    );
  }, []);

  useEffect(() => {
    void chat.me().then(setMe);
    loadTeams();
  }, [loadTeams]);

  // Open the channel of the project in this window, when there is one.
  const projectChannel = useMemo(() => {
    const all = teams?.flatMap((t) => t.channels) ?? [];
    const id = snapshot.project.team?.projectId;
    return id ? all.find((c) => c.projectId === id) : undefined;
  }, [teams, snapshot.project.team?.projectId]);
  const choseProjectChannel = useRef(false);
  useEffect(() => {
    if (projectChannel && !choseProjectChannel.current) {
      choseProjectChannel.current = true;
      setSelected(projectChannel.id);
    }
  }, [projectChannel]);

  const markRead = useCallback((channel: string) => {
    void chat.read(channel).catch(() => {});
    setTeams((t) => t && t.map((team) => ({ ...team, channels: team.channels.map((c) => (c.id === channel ? { ...c, unread: 0 } : c)) })));
  }, []);

  useEffect(() => {
    if (!selected) return;
    setThreads([]);
    setReplying(null);
    chat.posts(selected, null).then(
      (page) => {
        setThreads(page.threads);
        setHasMore(page.hasMore);
        markRead(selected);
        requestAnimationFrame(() => bottom.current?.scrollIntoView());
      },
      (e) => setProblem(errorMessage(e)),
    );
  }, [selected, markRead]);

  // New posts and channels as they happen.
  useEffect(() => {
    const off = listen<ChatEvent>("chat-event", (e) => {
      const ev = e.payload;
      if (ev.kind === "channel") {
        loadTeams();
        return;
      }
      const p = ev.post;
      if (p.channelId === selectedRef.current) {
        setThreads((list) => {
          if (list.some((t) => t.post.id === p.id || t.replies.some((r) => r.id === p.id))) return list;
          if (p.parentId) return list.map((t) => (t.post.id === p.parentId ? { ...t, replies: [...t.replies, p] } : t));
          return [{ post: p, replies: [] }, ...list];
        });
        markRead(p.channelId);
        requestAnimationFrame(() => bottom.current?.scrollIntoView({ behavior: "smooth" }));
      } else if (p.authorId !== me) {
        setTeams((t) => t && t.map((team) => ({ ...team, channels: team.channels.map((c) => (c.id === p.channelId ? { ...c, unread: c.unread + 1 } : c)) })));
      }
    });
    return () => void off.then((u) => u());
  }, [loadTeams, markRead, me]);

  const send = async (body: string, parentId: string | null) => {
    if (!selected) return;
    try {
      const p = await chat.send(selected, body, parentId);
      // The live stream brings it too; adding it now keeps the conversation snappy.
      setThreads((list) => {
        if (list.some((t) => t.post.id === p.id || t.replies.some((r) => r.id === p.id))) return list;
        return parentId ? list.map((t) => (t.post.id === parentId ? { ...t, replies: [...t.replies, p] } : t)) : [{ post: p, replies: [] }, ...list];
      });
      if (!parentId) requestAnimationFrame(() => bottom.current?.scrollIntoView({ behavior: "smooth" }));
    } catch (e) {
      setProblem(errorMessage(e));
      throw e;
    }
  };

  const loadOlder = () => {
    if (!selected || threads.length === 0) return;
    chat.posts(selected, threads[threads.length - 1].post.id).then(
      (page) => {
        setThreads((list) => [...list, ...page.threads]);
        setHasMore(page.hasMore);
      },
      (e) => setProblem(errorMessage(e)),
    );
  };

  const addChannel = async (teamId: string) => {
    try {
      const c = await chat.createChannel(teamId, newChannel);
      setAdding(null);
      setNewChannel("");
      loadTeams();
      setSelected(c.id);
    } catch (e) {
      setProblem(errorMessage(e));
    }
  };

  if (teams === null && problem) {
    return (
      <div className="chat-empty">
        <h2>Chat with your team</h2>
        <p>{problem}</p>
        <p className="secondary">Create a team project (Team &gt; Create Team Project) or join a team (Team &gt; Join a Team) to talk with your team here.</p>
      </div>
    );
  }
  const channel = teams?.flatMap((t) => t.channels).find((c) => c.id === selected);
  const team = teams?.find((t) => t.channels.some((c) => c.id === selected));
  const isProjectHere = !!channel?.projectId && channel.id === projectChannel?.id;
  // Oldest conversation first, as in Teams.
  const ordered = [...threads].reverse();

  return (
    <div className="chat">
      <nav className="chat-teams" aria-label="Teams and channels">
        {(teams ?? []).map((t) => (
          <section key={t.id}>
            <h3>{t.name}</h3>
            <ul>
              {t.channels.map((c) => (
                <li key={c.id}>
                  <button className={`channel ${c.id === selected ? "on" : ""} ${c.unread ? "unread" : ""}`} onClick={() => setSelected(c.id)} aria-current={c.id === selected}>
                    <span className="channel-name">
                      {c.projectId ? "▤ " : "# "}
                      {c.name}
                    </span>
                    {c.unread > 0 && <span className="badge">{c.unread}</span>}
                  </button>
                </li>
              ))}
            </ul>
            {adding === t.id ? (
              <form
                className="add-channel"
                onSubmit={(e) => {
                  e.preventDefault();
                  void addChannel(t.id);
                }}
              >
                <input aria-label="Channel name" placeholder="Channel name" value={newChannel} onChange={(e) => setNewChannel(e.target.value)} autoFocus />
              </form>
            ) : (
              <button className="link small" onClick={() => setAdding(t.id)}>
                + Add channel
              </button>
            )}
          </section>
        ))}
      </nav>
      <section className="chat-channel" aria-label={channel ? `Channel ${channel.name}` : "Channel"}>
        {channel && (
          <header>
            <h2>
              {channel.projectId ? "▤ " : "# "}
              {channel.name}
            </h2>
            <span className="secondary small">
              {team?.name}
              {channel.projectId ? " · the project's channel" : ""} · @mention people · #12 links to task 12
            </span>
          </header>
        )}
        {problem && (
          <p role="alert" className="banner error">
            {problem}
          </p>
        )}
        <div className="chat-scroll">
          {hasMore && (
            <button className="link chat-older" onClick={loadOlder}>
              Show older conversations
            </button>
          )}
          {channel && ordered.length === 0 && <p className="secondary chat-none">No conversations yet. Start one below.</p>}
          {ordered.map((t) => (
            <article key={t.post.id} className="thread">
              <Author post={t.post} mine={t.post.authorId === me} />
              <Body text={t.post.body} onTask={isProjectHere ? onShowTask : null} rowCount={snapshot.rows.length} />
              {t.replies.length > 0 && (
                <div className="replies">
                  {t.replies.map((r) => (
                    <div key={r.id} className="reply">
                      <Author post={r} mine={r.authorId === me} />
                      <Body text={r.body} onTask={isProjectHere ? onShowTask : null} rowCount={snapshot.rows.length} />
                    </div>
                  ))}
                </div>
              )}
              {replying === t.post.id ? (
                <div className="replies">
                  <Composer placeholder="Reply" autoFocus onSend={(b) => send(b, t.post.id)} />
                </div>
              ) : (
                <button className="link small reply-link" onClick={() => setReplying(t.post.id)}>
                  ↩ Reply
                </button>
              )}
            </article>
          ))}
          <div ref={bottom} />
        </div>
        {channel && <Composer placeholder={`Start a new conversation in ${channel.name}`} onSend={(b) => send(b, null)} />}
      </section>
    </div>
  );
}
