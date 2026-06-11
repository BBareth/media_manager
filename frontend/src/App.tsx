import { useCallback, useEffect, useState } from "react";
import { Job, listJobs } from "./api";
import CompressTab from "./components/CompressTab";
import DownloadTab from "./components/DownloadTab";
import TranscodeTab from "./components/TranscodeTab";

type Tab = "download" | "transcode" | "compress";

export default function App() {
  const [tab, setTab] = useState<Tab>("download");
  const [jobs, setJobs] = useState<Job[]>([]);
  const [connected, setConnected] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const data = await listJobs();
      setJobs(data);
      setConnected(true);
    } catch {
      setConnected(false);
    }
  }, []);

  // Poll while there is work in flight; otherwise poll slowly.
  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 1500);
    return () => clearInterval(id);
  }, [refresh]);

  const downloadJobs = jobs.filter((j) => j.kind === "download");
  const transcodeJobs = jobs.filter((j) => j.kind === "transcode");
  const compressJobs = jobs.filter((j) => j.kind === "compress");

  return (
    <div className="app">
      <header className="app-header">
        <div className="brand">
          <span className="logo">▶</span>
          <h1>Media Manager</h1>
        </div>
        <div className={`status-dot ${connected ? "online" : "offline"}`} title={connected ? "Connected" : "Backend unreachable"} />
      </header>

      <nav className="tabs">
        <button
          className={tab === "download" ? "tab active" : "tab"}
          onClick={() => setTab("download")}
        >
          Download
          {downloadJobs.length > 0 && <span className="count">{downloadJobs.length}</span>}
        </button>
        <button
          className={tab === "transcode" ? "tab active" : "tab"}
          onClick={() => setTab("transcode")}
        >
          Transcode
          {transcodeJobs.length > 0 && <span className="count">{transcodeJobs.length}</span>}
        </button>
        <button
          className={tab === "compress" ? "tab active" : "tab"}
          onClick={() => setTab("compress")}
        >
          Compress
          {compressJobs.length > 0 && <span className="count">{compressJobs.length}</span>}
        </button>
      </nav>

      <main className="content">
        {tab === "download" ? (
          <DownloadTab jobs={downloadJobs} onChanged={refresh} />
        ) : tab === "transcode" ? (
          <TranscodeTab jobs={transcodeJobs} onChanged={refresh} />
        ) : (
          <CompressTab jobs={compressJobs} onChanged={refresh} />
        )}
      </main>

      <footer className="app-footer">
        Files are deleted automatically about a day after they are created. Download what you need, then delete it to free space.
      </footer>
    </div>
  );
}
