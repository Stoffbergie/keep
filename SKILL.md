---
name: keep
description: Use the keep CLI or API to move files, logs, archives or stdin from one machine to another through a permanent link. Use when a task asks to upload or download with keep, transfer files between machines, share an encrypted file, list or delete keep uploads, or verify an upload and download round trip. Do not use for developing or releasing the keep repo.
---

# keep

Upload bytes from a terminal, get a link, download the same bytes somewhere else. Files never expire.

## Install or check

```bash
command -v keep || brew install stoffbergie/tap/keep
keep --version
```

The default server is `https://keep.stoff.dev`. Set `KEEP_URL` for another one.

## Upload

```bash
keep ./report.pdf
cat deploy.log | keep -n deploy.log
keep -e ./secret.env
```

The link is the only thing printed on stdout, so `url=$(keep ./file)` works. With `-e` the link ends in `#<key>`; keep the whole link.

## Download

```bash
keep get "$url"
keep get "$url" -o ./downloads/
keep get "$url" -o - > ./file
```

Quote links that contain `#` so the shell keeps the key.

## Transfer between machines

When a remote command runner has timeouts or mangles binary output, upload locally, download on the remote side, then compare hashes. Downloads only appear under their final name once complete.

Local:

```bash
shasum -a 256 ./file.mp4
url=$(keep ./file.mp4)
```

Remote:

```bash
keep get "$url" -o ./file.mp4
shasum -a 256 ./file.mp4
```

For long downloads on a runner with short timeouts, run the download in the background and poll a log file:

```bash
log="/tmp/keep-$(date +%s).log"
( keep get "$url" -o ./file.mp4 && shasum -a 256 ./file.mp4 ) > "$log" 2>&1 &
```

## List and delete

```bash
keep ls
keep rm "$url"
```

Only uploads made from the current machine can be listed and deleted.

## Round trip check

```bash
dir=$(mktemp -d)
printf 'keep check\n' > "$dir/in.txt"
url=$(keep "$dir/in.txt")
keep get "$url" -o "$dir/out.txt"
cmp "$dir/in.txt" "$dir/out.txt"
keep rm "$url"
rm -rf "$dir"
```

## Safety

- Use `-e` for secrets.
- Uploads are public to anyone with the link.
- Do not paste secret file contents into chat. Upload with `-e` and pass the link.
