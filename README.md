# keep

Files from your terminal, kept forever.

```console
$ keep report.pdf
https://keep.stoff.dev/x7k2mzq4

$ keep get https://keep.stoff.dev/x7k2mzq4
report.pdf
```

No accounts and no expiry. A file stays until you delete it.

## Install

```bash
brew install stoffbergie/tap/keep
```

Or build from source with `cargo install --path cli`.

## Use

| Command | What it does |
| --- | --- |
| `keep <file>` | Upload a file and print its link. |
| `cat build.log \| keep -n build.log` | Upload stdin under a name. |
| `keep -e <file>` | Encrypt on this machine first. The key lives after `#` in the link and never reaches the server. |
| `keep get <link>` | Download. In a terminal it saves under the original name, in a pipe it writes to stdout. Decrypts when the link has a key. |
| `keep get <link> -o <path>` | Save to a file or directory, or `-` for stdout. |
| `keep ls` | List uploads made from this machine. |
| `keep rm <link>` | Delete an upload made from this machine. |

Set `KEEP_URL` to use your own server.

Files over 32 MB upload in parts, four at a time. Every request retries on network errors, and downloads only appear under their final name once complete.

## curl

```bash
curl -T report.pdf https://keep.stoff.dev
curl -OJ https://keep.stoff.dev/x7k2mzq4
curl -X DELETE -H "Authorization: Bearer $token" https://keep.stoff.dev/x7k2mzq4
```

Single requests are limited to 100 MB. The delete token comes back in the `X-Delete-Token` header of the upload (`curl -i`).

## API

| Method | Path | |
| --- | --- | --- |
| `PUT` | `/:filename` | Upload. Responds `201` with the link as text and an `X-Delete-Token` header. Send `X-Encrypted: true` for encrypted bytes. |
| `GET` | `/:id` | File bytes. Browsers get a download page instead. |
| `GET` | `/:id/raw` | File bytes, always. |
| `DELETE` | `/:id` | Delete with `Authorization: Bearer <token>`. |
| `POST` | `/api/multipart?filename=` | Start a large upload. Returns `{ id, uploadId }`. |
| `PUT` | `/api/multipart/:id/:part` | Upload one part with `X-Upload-Id`. Returns `{ partNumber, etag }`. |
| `POST` | `/api/multipart/:id` | Finish with `X-Upload-Id` and `{ "parts": [...] }`. Responds like `PUT /:filename`. |
| `GET` | `/health` | `{ "ok": true }` |

## How it works

One Cloudflare Worker in front of one R2 bucket. Each file is one R2 object, with its filename, content type and delete token in the object's metadata. There is no database and nothing expires.

Encryption is AES-256-GCM on the client. The download page decrypts in the browser with the key from the link, so encrypted links work without the CLI.

Uploads live in the `keep-files` bucket, with their original upload dates preserved.

## Develop

```bash
pnpm install
pnpm dev     # Worker on http://localhost:8787 with local storage
pnpm check   # TypeScript, rustfmt and clippy
pnpm test    # the real CLI against a local Worker
```

## Self-host

1. Create a bucket with `pnpm wrangler r2 bucket create keep-files`.
2. In `wrangler.toml`, set `bucket_name` and point `routes` at your domain.
3. Run `pnpm wrangler deploy`.
4. Run `export KEEP_URL=https://your.domain` wherever you use the CLI.

## License

MIT
