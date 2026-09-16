# Contributing

Bug reports, fixes, and small well-scoped features are all welcome.

## Before a large change

Open an issue first. This is a deliberately small tool — three tabs, one binary,
no database, no accounts — and the most useful thing a proposal can do is
explain why it belongs *inside* that shape. A change that would add a dependency
on a service, a login system, or a persistent library of media is probably a
different project rather than a patch to this one.

## Getting set up

You need Rust (stable), Node 20+, and `ffmpeg`, `ffprobe`, and `yt-dlp` on your
`PATH`.

```bash
# terminal 1 — backend on :8080
cd backend && cargo run

# terminal 2 — frontend on :5173, proxying /api to :8080
cd frontend && npm install && npm run dev
```

If you would rather not install the media tools locally, `docker compose up
--build` runs the whole thing; it is a slower loop but needs nothing on the
host.

## What CI checks

Run these before pushing and there will be no surprises:

```bash
cd backend
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test

cd ../frontend
npx tsc --noEmit
npm run build
```

CI also builds the Docker image and smoke-tests it: the image must contain
working `yt-dlp`, `ffmpeg`, and `ffprobe`, serve the SPA, answer
`/api/health`, and reject a `file://` download.

## House style

- **Match the surrounding code.** It is consistent; please keep it that way.
- **Comments explain why, not what.** The existing comments are load-bearing —
  they record a measurement, a failure that was hit in production, or the reason
  an obvious-looking alternative was rejected. A comment that restates the line
  below it is noise; one that says "the standalone yt-dlp binary fails on a
  constrained `/tmp`" saves the next person an afternoon.
- **Claims about performance need a number.** The encoding ladder exists because
  it was measured. If you change how encoding decisions are made, say what you
  measured, on what source, and on what hardware.
- **Keep the frontend dependency-free.** React and Vite, nothing else. No
  component library, no state manager, no CSS framework.
- **Never build a filesystem path out of a name that came from a request.**
  See [SECURITY.md](./SECURITY.md).

## Commits and pull requests

Write commit subjects in the imperative — "reject non-http download URLs", not
"rejected" or "fixes". Explain the reasoning in the body when it is not obvious
from the diff.

One concern per pull request. A pull request that fixes a bug and also
reformats four unrelated files is hard to review and hard to revert.

By contributing you agree that your contribution is licensed under the
[MIT License](./LICENSE).
