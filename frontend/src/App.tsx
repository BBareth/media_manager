import { KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { Job, JobKind, ServerInfo, getInfo } from "./api";
import { formatPeriod, shortTitle } from "./format";
import { oneOf, usePersistentState } from "./hooks/usePersistentState";
import { Connection, useJobs } from "./hooks/useJobs";
import CompressTab from "./components/CompressTab";
import DownloadTab from "./components/DownloadTab";
import Icon, { IconName, Logo } from "./components/Icon";
import TranscodeTab from "./components/TranscodeTab";

const TABS: { kind: JobKind; label: string; icon: IconName }[] = [
  { kind: "download", label: "Download", icon: "download" },
  { kind: "transcode", label: "Convert", icon: "convert" },
  { kind: "compress", label: "Compress", icon: "compress" },
];
const KINDS = TABS.map((t) => t.kind);

const isActive = (j: Job) => j.status === "running" || j.status === "queued";

export default function App() {
  const [tab, setTab] = usePersistentState<JobKind>("tab", "download", oneOf(KINDS));
  const { jobs, connection, addJob, removeJobs } = useJobs();
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const announcement = useAnnouncements(jobs);
  const tabRefs = useRef<Record<string, HTMLButtonElement | null>>({});

  useEffect(() => {
    getInfo().then(setInfo, () => {});
  }, []);

  const byKind = useMemo(() => {
    const groups: Record<JobKind, Job[]> = { download: [], transcode: [], compress: [] };
    for (const j of jobs) groups[j.kind].push(j);
    return groups;
  }, [jobs]);

  // Show work in progress in the tab title, for when this page sits in a background tab.
  const activeCount = jobs.filter(isActive).length;
  useEffect(() => {
    document.title = activeCount > 0 ? `(${activeCount}) Media Manager` : "Media Manager";
  }, [activeCount]);

  // Arrow keys move between tabs, as the WAI-ARIA tabs pattern expects.
  function onTabKey(e: KeyboardEvent) {
    const i = KINDS.indexOf(tab);
    const next =
      e.key === "ArrowRight" ? (i + 1) % KINDS.length
      : e.key === "ArrowLeft" ? (i - 1 + KINDS.length) % KINDS.length
      : e.key === "Home" ? 0
      : e.key === "End" ? KINDS.length - 1
      : -1;
    if (next < 0) return;
    e.preventDefault();
    setTab(KINDS[next]);
    tabRefs.current[KINDS[next]]?.focus();
  }

  const panelProps = { jobs: byKind[tab], onJob: addJob, onDelete: removeJobs };

  return (
    <div className="app">
      <header className="app-header">
        <div className="brand">
          <Logo size={40} />
          <div>
            <h1>Media Manager</h1>
            <p className="tagline">Download, convert and compress — on your own machine.</p>
          </div>
        </div>
        <ConnectionStatus connection={connection} />
      </header>

      {connection === "offline" && (
        <div className="banner" role="status">
          <Icon name="alert" size={18} />
          Lost contact with the server. Jobs keep running there; this page reconnects on its own.
        </div>
      )}

      <div className="tabs" role="tablist" aria-label="Tools" onKeyDown={onTabKey}>
        {TABS.map((t) => {
          const group = byKind[t.kind];
          const active = group.filter(isActive).length;
          const ready = group.some((j) => j.status === "completed");
          return (
            <button
              key={t.kind}
              ref={(el) => {
                tabRefs.current[t.kind] = el;
              }}
              id={`tab-${t.kind}`}
              role="tab"
              type="button"
              aria-selected={tab === t.kind}
              aria-controls={`panel-${t.kind}`}
              tabIndex={tab === t.kind ? 0 : -1}
              className="tab"
              onClick={() => setTab(t.kind)}
            >
              <Icon name={t.icon} size={18} />
              <span>{t.label}</span>
              {active > 0 ? (
                <span className="tab-count" aria-label={`${active} in progress`}>
                  {active}
                </span>
              ) : ready && tab !== t.kind ? (
                <span className="tab-dot" aria-label="has finished files" />
              ) : null}
            </button>
          );
        })}
      </div>

      <main id={`panel-${tab}`} role="tabpanel" aria-labelledby={`tab-${tab}`} className="panel">
        {tab === "download" ? (
          <DownloadTab {...panelProps} />
        ) : tab === "transcode" ? (
          <TranscodeTab {...panelProps} />
        ) : (
          <CompressTab {...panelProps} />
        )}
      </main>

      <footer className="app-footer">
        {info ? (
          <>
            Files are deleted {formatPeriod(info.retention_secs)} after they are created — save what you need.{" "}
            {info.max_encodes === 1
              ? "One conversion runs at a time; the rest wait their turn."
              : `Up to ${info.max_encodes} conversions run at once; the rest wait their turn.`}
          </>
        ) : (
          "Files are deleted automatically after a while — save what you need."
        )}
      </footer>

      <div className="sr-only" aria-live="polite" role="status">
        {announcement}
      </div>
    </div>
  );
}

function ConnectionStatus({ connection }: { connection: Connection }) {
  const label = connection === "live" ? "Live" : connection === "connecting" ? "Connecting…" : "Reconnecting…";
  return (
    <div className={`connection connection-${connection}`} title={connection === "live" ? "Receiving live updates" : undefined}>
      <span className="connection-dot" aria-hidden="true" />
      {label}
    </div>
  );
}

/// A sentence for screen readers whenever a job finishes or fails, since the change is otherwise
/// only visual and may happen on another tab of the app.
function useAnnouncements(jobs: Job[]): string {
  const previous = useRef<Map<string, Job["status"]> | null>(null);
  const [message, setMessage] = useState("");

  useEffect(() => {
    const before = previous.current;
    previous.current = new Map(jobs.map((j) => [j.id, j.status]));
    if (!before) return; // the first list is history, not news
    const news = jobs
      .filter((j) => before.has(j.id) && before.get(j.id) !== j.status)
      .map((j) => {
        const name = j.output_name ?? shortTitle(j.title);
        return j.status === "completed" ? `${name} is ready.` : j.status === "failed" ? `${name} failed.` : null;
      })
      .filter(Boolean);
    if (news.length > 0) setMessage(news.join(" "));
  }, [jobs]);

  return message;
}
