import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { chat, errorMessage, team as teamApi, type Snapshot } from "./api";
import type { ChatChannel } from "./bindings/ChatChannel";
import type { ChatPost } from "./bindings/ChatPost";
import type { ChatTeam } from "./bindings/ChatTeam";
import type { ChatThread } from "./bindings/ChatThread";

/** Puts a new or changed post in its place: a conversation, or a reply in one. */
function upsert(list: ChatThread[], p: ChatPost): ChatThread[] {
  if (p.parentId) {
    return list.map((t) =>
      t.post.id !== p.parentId ? t : { ...t, replies: t.replies.some((r) => r.id === p.id) ? t.replies.map((r) => (r.id === p.id ? p : r)) : [...t.replies, p] },
    );
  }
  if (list.some((t) => t.post.id === p.id)) return list.map((t) => (t.post.id === p.id ? { ...t, post: p } : t));
  return [{ post: p, replies: [] }, ...list];
}

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

const REACTIONS = ["👍", "❤️", "😄", "🎉", "👀", "✅"];

/** One post or reply: who, when, the text, reactions, and (on hover) react / edit / delete. */
function PostView({
  post,
  me,
  onTask,
  rowCount,
  onChanged,
  onError,
  onConfirm,
}: {
  post: ChatPost;
  me: string | null;
  onTask: ((row: number) => void) | null;
  rowCount: number;
  onChanged: (p: ChatPost) => void;
  onError: (message: string) => void;
  onConfirm: (title: string, message: string, action: string) => Promise<boolean>;
}) {
  const mine = post.authorId === me;
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(post.body);
  const run = (p: Promise<ChatPost>) => p.then(onChanged, (e) => onError(errorMessage(e)));
  // Reactions grouped by emoji, in the order first used.
  const groups = new Map<string, { names: string[]; mine: boolean }>();
  for (const r of post.reactions) {
    const g = groups.get(r.emoji) ?? { names: [], mine: false };
    g.names.push(r.userName);
    g.mine ||= r.userId === me;
    groups.set(r.emoji, g);
  }
  return (
    <div className="post">
      <div className="chat-meta">
        <span className="avatar" aria-hidden>
          {post.authorName.slice(0, 1).toUpperCase()}
        </span>
        <strong>{mine ? `${post.authorName} (you)` : post.authorName}</strong>
        <span className="secondary small">
          {when(post.at)}
          {post.editedAt && !post.deleted ? " · edited" : ""}
        </span>
        {!post.deleted && !editing && (
          <span className="post-actions">
            {REACTIONS.map((e) => (
              <button key={e} className="react" title={`React with ${e}`} aria-label={`React with ${e}`} onClick={() => void run(chat.react(post.id, e))}>
                {e}
              </button>
            ))}
            {mine && (
              <button className="link small" onClick={() => setEditing(true)}>
                Edit
              </button>
            )}
            <button
              className="link small"
              onClick={() => void onConfirm("Delete this post?", "It is removed for everyone. Replies to it stay.", "Delete").then((yes) => {
                  if (yes) void run(chat.remove(post.id));
                })
              } title={mine ? "Delete your post" : "Delete (team owners can delete any post)"}>
              Delete
            </button>
          </span>
        )}
      </div>
      {post.deleted ? (
        <div className="chat-body secondary">This post was deleted.</div>
      ) : editing ? (
        <form
          className="composer chat-edit"
          onSubmit={(e) => {
            e.preventDefault();
            void run(chat.edit(post.id, text)).then(() => setEditing(false));
          }}
        >
          <textarea value={text} onChange={(e) => setText(e.target.value)} rows={Math.min(6, Math.max(1, text.split("\n").length))} autoFocus aria-label="Edit post" />
          <button type="button" onClick={() => setEditing(false)}>
            Cancel
          </button>
          <button className="primary" type="submit">
            Save
          </button>
        </form>
      ) : (
        <Body text={post.body} onTask={onTask} rowCount={rowCount} />
      )}
      {groups.size > 0 && (
        <div className="reactions">
          {[...groups].map(([emoji, g]) => (
            <button key={emoji} className={g.mine ? "chip mine" : "chip"} title={g.names.join(", ")} onClick={() => void run(chat.react(post.id, emoji))}>
              {emoji} {g.names.length}
            </button>
          ))}
        </div>
      )}
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
export function ChatView({
  snapshot,
  onShowTask,
  onConfirm,
}: {
  snapshot: Snapshot;
  onShowTask: (row: number) => void;
  onConfirm: (title: string, message: string, action: string) => Promise<boolean>;
}) {
  const [teams, setTeams] = useState<ChatTeam[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [threads, setThreads] = useState<ChatThread[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [me, setMe] = useState<string | null>(null);
  const [meName, setMeName] = useState<string | null>(null);
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
    void teamApi.status().then((s) => setMeName(s.user?.name ?? null));
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
        setThreads((list) => upsert(list, p));
        markRead(p.channelId);
        requestAnimationFrame(() => bottom.current?.scrollIntoView({ behavior: "smooth" }));
      } else if (p.authorId !== me && !p.editedAt && !p.deleted && p.reactions.length === 0) {
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
      setThreads((list) => upsert(list, p));
      if (!parentId) requestAnimationFrame(() => bottom.current?.scrollIntoView({ behavior: "smooth" }));
    } catch (e) {
      setProblem(errorMessage(e));
      throw e;
    }
  };

  const changed = (p: ChatPost) => setThreads((list) => upsert(list, p));
  /** "@Alex" mentions the person whose first name is Alex. */
  const mentionsMe = (body: string) => {
    const first = meName?.split(" ")[0]?.toLowerCase();
    if (!first) return false;
    return (body.toLowerCase().match(/@[\p{L}\p{N}._-]+/gu) ?? []).some((m) => m.slice(1) === first);
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
            <article key={t.post.id} className={[t.post, ...t.replies].some((p) => mentionsMe(p.body)) ? "thread mentioned" : "thread"}>
              <PostView post={t.post} me={me} onTask={isProjectHere ? onShowTask : null} rowCount={snapshot.rows.length} onChanged={changed} onError={setProblem} onConfirm={onConfirm} />
              {t.replies.length > 0 && (
                <div className="replies">
                  {t.replies.map((r) => (
                    <div key={r.id} className="reply">
                      <PostView post={r} me={me} onTask={isProjectHere ? onShowTask : null} rowCount={snapshot.rows.length} onChanged={changed} onError={setProblem} onConfirm={onConfirm} />
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
