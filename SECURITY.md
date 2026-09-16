# Security

## The trust model, stated plainly

media_manager is designed for a **single trusted user or a trusted LAN**. It is
not hardened for hostile input, and the following are deliberate properties of
the current design rather than oversights:

- **There is no authentication or authorization.** Anyone who can reach the port
  can start jobs, list every job, download any finished file, and delete
  anything. Jobs are not scoped to a user, because there are no users.
- **CORS is permissive.** Any website you visit while the server is reachable
  from your browser can issue API calls to it. Combined with the absence of
  auth, this means "reachable" should mean "reachable by you only".
- **The server fetches URLs you give it.** Requests originate from the server,
  so anything the container can reach on your network is reachable through the
  download endpoint. Only `http://` and `https://` are accepted — `file://` and
  other schemes are rejected — but that does not make it an SSRF-safe proxy.
- **Uploads are large and unauthenticated.** The transcode and compress
  endpoints accept up to 50 GiB per request and write to the data volume. A
  reachable instance can be made to fill its disk.
- **Transcoding hostile media is transcoding hostile media.** `ffmpeg` and
  `yt-dlp` are large, capable programs that parse untrusted formats. Run them
  in the container, not on your desktop.

**Do not expose this directly to the internet.** If you need remote access, put
it behind something that authenticates first — a VPN such as WireGuard or
Tailscale, an authenticating reverse proxy, or Cloudflare Access — and keep the
container's own port bound to localhost or a private interface.

## What is guarded

Where an attack is cheap to prevent, it is prevented:

- Subprocesses are spawned with an argument vector, never a shell string, so
  there is no shell to inject into.
- The URL is passed after a `--` terminator so that a value beginning with a
  dash cannot be reinterpreted as a `yt-dlp` option such as `--exec`.
- Download formats and quality values are checked against fixed allowlists.
- Each job writes only inside its own UUID-named directory, and outputs are
  served from a path recorded server-side rather than from anything in the
  request.
- Filenames from uploads and from remote metadata are never used to build the
  path that is written to; they only become a `Content-Disposition` suggestion,
  escaped for that header.

## Supported versions

Fixes land on `main` and in the next tagged release. Only the latest release is
supported; there are no backports.

## Reporting a vulnerability

Please report privately, not in a public issue:

**<https://github.com/BBareth/media_manager/security/advisories/new>**

Include what an attacker can do, how to reproduce it, and the version. You can
expect a first reply within about a week. This is a hobby project maintained in
spare time — there is no bounty, and no SLA beyond a good-faith effort to fix
real problems and credit you in the advisory.

Issues that amount to "an unauthenticated instance exposed to the internet can
be abused" are documented above and are not vulnerabilities in themselves.
Something that lets an attacker escape the documented model — read files outside
the data directory, execute commands, or reach state belonging to another job —
is, and I want to hear about it.
