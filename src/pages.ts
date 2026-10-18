import type { StoredFile } from "./index"
import { imageType } from "./previews"

const SOURCE_URL = "https://github.com/Stoffbergie/keep"
const TAGLINE = "Files from your terminal, kept forever."

const STYLE = `
:root{--bg:#fafafa;--fg:#171717;--muted:#737373;color-scheme:light dark}
@media (prefers-color-scheme:dark){:root{--bg:#0a0a0a;--fg:#ededed;--muted:#8f8f8f}}
*{box-sizing:border-box;margin:0;padding:0}
html{background:var(--bg);color:var(--fg);font:14px/1.7 ui-monospace,SFMono-Regular,"SF Mono",Menlo,Consolas,"Liberation Mono",monospace;-webkit-text-size-adjust:100%}
body{max-width:46rem;margin:0 auto;padding:14vh 1.5rem 6rem}
a{color:inherit;text-decoration-color:var(--muted);text-underline-offset:3px}
header{margin-bottom:3.5rem}
header a{font-weight:600;text-decoration:none}
h1{font-size:1rem;font-weight:600;overflow-wrap:anywhere}
h2{margin:3rem 0 .75rem;color:var(--muted);font-size:.75rem;font-weight:400;letter-spacing:.08em;text-transform:uppercase}
p{color:var(--muted)}
dd{margin-bottom:1rem;color:var(--muted)}
dt,code{font:inherit;overflow-wrap:anywhere;user-select:all}
footer{margin-top:4rem}
.commands{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:.8rem 2rem}
.commands dd{margin:0}
details{margin-top:2.5rem}
summary{color:var(--muted);cursor:pointer}
@media(max-width:600px){.commands{grid-template-columns:1fr;gap:0}.commands dd{margin-bottom:1rem}}
.button{display:inline-block;margin:2.5rem 0 0;padding:.55rem 1.4rem;border:0;border-radius:3px;background:var(--fg);color:var(--bg);font:inherit;text-decoration:none;cursor:pointer}
.button:hover{opacity:.85}
.button:disabled{opacity:.35;cursor:default}
#status{min-height:1.7em;margin-top:1rem}
:focus-visible{outline:2px solid var(--fg);outline-offset:3px}
`

const DECRYPT_SCRIPT = `
const button = document.getElementById("download")
const status = document.getElementById("status")
const key = location.hash.slice(1).replace(/^key=/, "")
const fromBase64 = (value) => Uint8Array.from(atob(value.replace(/-/g, "+").replace(/_/g, "/")), (char) => char.charCodeAt(0))
const importKey = async () => {
  const bytes = fromBase64(key)
  if (bytes.length !== 32) throw new Error("Invalid key")
  return crypto.subtle.importKey("raw", bytes, "AES-GCM", false, ["decrypt"])
}
const decrypt = async (data) => {
  return crypto.subtle.decrypt({ name: "AES-GCM", iv: data.subarray(0, 12) }, await importKey(), data.subarray(12))
}
const decryptFile = async (response) => {
  const reader = response.body.getReader()
  let buffer = new Uint8Array(0)
  let ended = false
  const fill = async (wanted) => {
    while (!ended && buffer.length < wanted) {
      const next = await reader.read()
      if (next.done) {
        ended = true
        break
      }
      const joined = new Uint8Array(buffer.length + next.value.length)
      joined.set(buffer)
      joined.set(next.value, buffer.length)
      buffer = joined
    }
  }
  try {
    await fill(15)
    if (buffer.length < 15 || new TextDecoder().decode(buffer.subarray(0, 8)) !== "keep-v2\\n") {
      await fill(Infinity)
      return [await decrypt(buffer)]
    }
    const prefix = buffer.slice(8, 15)
    buffer = buffer.slice(15)
    const cryptoKey = await importKey()
    const parts = []
    for (let index = 0; ; index++) {
      await fill(1048576 + 17)
      const length = Math.min(buffer.length, 1048576 + 16)
      const last = buffer.length <= 1048576 + 16
      const iv = new Uint8Array(12)
      iv.set(prefix)
      new DataView(iv.buffer).setUint32(7, index)
      iv[11] = last ? 1 : 0
      parts.push(await crypto.subtle.decrypt({ name: "AES-GCM", iv }, cryptoKey, buffer.subarray(0, length)))
      buffer = buffer.slice(length)
      if (last) return parts
    }
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}
if (key) {
  document.getElementById("command").textContent = "keep get '" + location.href + "'"
  status.textContent = "Checking key…"
  const prepare = async () => {
    await importKey()
    if (button.dataset.encryptedName) {
      const plain = await decrypt(fromBase64(button.dataset.encryptedName))
      const name = new TextDecoder().decode(plain)
      document.querySelector("h1").textContent = name
      document.title = name + " · keep"
      button.dataset.filename = name
    }
    button.disabled = false
    status.textContent = ""
  }
  prepare().catch(() => {
    status.textContent = "Could not decrypt. Check that the link is complete."
  })
  button.addEventListener("click", async (event) => {
    event.preventDefault()
    button.disabled = true
    status.textContent = "Decrypting…"
    try {
      const response = await fetch(button.dataset.raw)
      if (!response.ok) throw new Error(response.statusText)
      const parts = await decryptFile(response)
      const url = URL.createObjectURL(new Blob(parts))
      const link = document.createElement("a")
      link.href = url
      link.download = button.dataset.filename
      link.click()
      setTimeout(() => URL.revokeObjectURL(url), 60000)
      status.textContent = ""
    } catch {
      status.textContent = "Could not decrypt. Check that the link is complete."
    } finally {
      button.disabled = false
    }
  })
}
`

type Section = { title: string; rows: [command: string, description: string, label: string][] }
type Metadata = { description: string; url?: string; image?: string; imageAlt?: string; index?: boolean; imageType?: string }

function usage(origin: string): Section[] {
  return [
    { title: "Install", rows: [["brew install stoffbergie/tap/keep", "Install the keep CLI with Homebrew on macOS or Linux.", ""]] },
    {
      title: "Use",
      rows: [
        ["keep photo.jpg", "Upload a file. Explicit files take priority over piped input. Print a link and copy it in a terminal; captured stdout contains links only. Files stay until deleted unless a lifetime is chosen.", "Share a file"],
        ["keep", "Upload copied text, an image or files from the clipboard. With piped input, upload stdin instead. Terminal uploads copy the resulting links by default.", "Share your clipboard"],
        ["keep -e secret.txt", "Encrypt the file and its name on this machine. The key stays in the link fragment and never reaches the server. Share the complete link so the recipient can decrypt it.", "Encrypt a file"],
        ["keep get x7k2mzq4", "Download by code or full link. Save a file and print its path, choosing an unused name if needed. Encrypted files require the complete link including its key.", "Download a file"],
        ["keep ls", "List this machine's upload history, including filenames, links, encryption and expiry. This is local history, not a list of every file on the server.", "Your uploads"],
        ["keep rm x7k2mzq4", "Delete one or more links or codes using the delete tokens saved on the machine that uploaded them. Check the whole selection before deleting anything.", "Delete a file"],
      ],
    },
    {
      title: "More",
      rows: [
        ["keep report.pdf notes.txt", "Upload several files and print one link per file. Validate all inputs before uploading. A single name cannot be used for multiple files.", "Share several files"],
        ["keep -e --expires 1d --once .env", "Encrypt and delete after one day or the first content download, whichever comes first. Expiry accepts minutes, hours or days, such as 30m, 12h or 7d. The older --expires once spelling still works. Pages, HEAD requests and link previews do not count as downloads.", "One day or one download"],
        ["make 2>&1 | keep - -n build.log", "Read stdout and stderr from a command using explicit stdin (-), and upload it with the chosen filename. Do not combine stdin with file arguments.", "Share command output"],
        ["keep get x7k2mzq4 -o -", "Write file contents to stdout instead of saving a file. The -o option also accepts a file path or an existing directory. Keys and destinations are checked before retrieving contents.", "Download to stdout"],
        ["keep config --copy false", "Disable automatic link copying for terminal uploads. Running keep config without options shows all preferences.", "Turn off link copying"],
      ],
    },
    {
      title: "curl",
      rows: [
        [`curl -i -T report.pdf ${origin}`, "Upload up to 100 MB. The response contains the link and the X-Delete-Token header needed for deletion.", "Upload a file"],
        [`curl -i -T report.pdf -H 'X-Expires: 1d' -H 'X-Once: true' ${origin}`, "Upload with a deadline and a one-download limit. The first condition reached makes the file unavailable.", "One day or one download"],
        [`curl -OJ ${origin}/<id>`, "Download a public file and save it under its original filename. Use keep or a browser to decrypt encrypted links.", "Download a file"],
        ["curl -X DELETE -H 'Authorization: Bearer <token>' <link>", "Delete an upload with the token from its X-Delete-Token response header.", "Delete a file"],
      ],
    },
  ]
}

export function homeText(origin: string): string {
  const lines = [`keep: ${TAGLINE.toLowerCase()}`, ""]
  for (const section of usage(origin)) {
    lines.push(section.title.toUpperCase())
    const width = Math.max(...section.rows.map(([command]) => command.length))
    for (const [command, description] of section.rows) {
      const comment = description ? `  # ${description}` : ""
      lines.push(`  ${command.padEnd(width)}${comment}`.trimEnd())
    }
    lines.push("")
  }
  lines.push("Opening a page or link preview does not count as a download.", "")
  return lines.join("\n")
}

export function homePage(origin: string): string {
  const sections = usage(origin)
    .map((section) => {
      const rows = section.rows
        .map(([command, description, label]) => `<dt>${escapeHtml(command)}</dt><dd>${escapeHtml(label)}<span hidden> ${escapeHtml(description)}</span></dd>`)
        .join("")
      return `<h2>${section.title}</h2><dl class="commands">${rows}</dl>`
    })
  return layout(
    "keep · Share a file. Get a link.",
    `<h1>Share a file. Get a link.</h1>
${sections.slice(0, 2).join("")}
<details><summary>More options</summary>${sections.slice(2).join("")}</details>
<footer><p><a href="${SOURCE_URL}">Source</a></p></footer>
<script type="application/ld+json">${JSON.stringify({ "@context": "https://schema.org", "@type": "WebSite", name: "keep", url: `${origin}/`, description: TAGLINE })}</script>`,
    {
      description: `${TAGLINE} Upload, share and download files with optional encryption and expiry. No accounts.`,
      url: `${origin}/`,
      image: `${origin}/social/keep.png`,
      imageAlt: TAGLINE,
      index: true,
    },
  )
}

export function filePage(file: StoredFile, origin: string, excerpt = ""): string {
  const raw = `/${encodeURIComponent(file.id)}/raw`
  const details = [formatSize(file.size), file.uploaded.toISOString().slice(0, 10)]
  if (file.encrypted) {
    details.push("encrypted")
  }
  if (file.once) {
    details.push("deleted after the first download")
  }
  if (file.expires) {
    details.push(`expires ${file.expires.toISOString().slice(0, 16).replace("T", " ")} UTC`)
  }
  if (file.once && file.expires) {
    details.push("whichever comes first")
  }
  let name = file.filename
  let data = `id="download" class="button" data-raw="${raw}" data-filename="${escapeHtml(file.filename)}"`
  if (file.encryptedName) {
    name = "Encrypted file"
    data += ` data-encrypted-name="${escapeHtml(file.encryptedName)}"`
  }
  const button = file.encrypted
    ? `<button ${data} disabled>Download</button>`
    : `<a ${data} href="${raw}" download>Download</a>`
  const status = file.encrypted ? "This link is missing the key that decrypts the file." : ""
  const url = `${origin}/${encodeURIComponent(file.id)}`
  let title = `${file.filename} · keep`
  let description = `${details.join(" · ")}. ${excerpt || "Download the original file from keep."}`
  let image = `${origin}/social/file.png`
  let imageAlt = "A file shared with keep"
  if (file.encrypted) {
    title = `${file.filename} · encrypted · keep`
    if (file.encryptedName) {
      title = `${name} · keep`
    }
    description = `Encrypted file · ${formatSize(file.size)}. The complete link is needed to decrypt it on your device.`
    image = `${origin}/social/encrypted.png`
    imageAlt = "Encrypted file. Only the complete link can unlock it."
  } else if (imageType(file)) {
    image = `${url}/preview`
    imageAlt = file.filename
  }
  return layout(
    title,
    `<h1>${escapeHtml(name)}</h1>
<p>${details.join(" · ")}</p>
${button}
<p id="status" role="status">${status}</p>
<h2>Terminal</h2>
<code id="command">keep get ${escapeHtml(`${origin}/${file.id}`)}</code>
${file.encrypted ? `<script>${DECRYPT_SCRIPT}</script>` : ""}`,
    { description, url, image, imageAlt, imageType: imageType(file) },
  )
}

export function notFoundPage(): string {
  return layout(
    "Not found · keep",
    `<h1>Not found</h1>
<p>Nothing is kept at this address.</p>`,
    { description: "Nothing is kept at this address." },
  )
}

function layout(title: string, body: string, metadata: Metadata): string {
  const social = metadata.url && metadata.image
    ? `<link rel="canonical" href="${escapeHtml(metadata.url)}">
<meta property="og:type" content="website">
<meta property="og:site_name" content="keep">
<meta property="og:title" content="${escapeHtml(title)}">
<meta property="og:description" content="${escapeHtml(metadata.description)}">
<meta property="og:url" content="${escapeHtml(metadata.url)}">
<meta property="og:image" content="${escapeHtml(metadata.image)}">
<meta property="og:image:secure_url" content="${escapeHtml(metadata.image)}">
<meta property="og:image:type" content="${escapeHtml(metadata.imageType ?? "image/png")}">
${metadata.imageType ? "" : '<meta property="og:image:width" content="1200"><meta property="og:image:height" content="630">'}
<meta property="og:image:alt" content="${escapeHtml(metadata.imageAlt ?? title)}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="${escapeHtml(title)}">
<meta name="twitter:description" content="${escapeHtml(metadata.description)}">
<meta name="twitter:image" content="${escapeHtml(metadata.image)}">
<meta name="twitter:image:alt" content="${escapeHtml(metadata.imageAlt ?? title)}">`
    : ""
  return `<!doctype html>
<html lang="en" prefix="og: https://ogp.me/ns#">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>${escapeHtml(title)}</title>
<meta name="description" content="${escapeHtml(metadata.description)}">
<meta name="robots" content="${metadata.index ? "index, follow" : "noindex, nofollow"}">
${social}
<link rel="icon" href="/favicon.png" type="image/png">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<style>${STYLE}</style>
</head>
<body>
<header><a href="/">keep</a></header>
<main>
${body}
</main>
</body>
</html>`
}

function formatSize(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"]
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  const amount = unit === 0 ? String(value) : value.toFixed(1)
  return `${amount} ${units[unit]}`
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;")
}
