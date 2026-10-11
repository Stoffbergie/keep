import type { StoredFile } from "./index"

const SOURCE_URL = "https://github.com/Stoffberg/keep"
const TAGLINE = "Files from your terminal, kept forever."

const ICON =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Crect width='16' height='16' rx='3' fill='%23737373'/%3E%3C/svg%3E"

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
if (key) {
  document.getElementById("command").textContent = "keep get '" + location.href + "'"
  button.disabled = false
  status.textContent = ""
  button.addEventListener("click", async (event) => {
    event.preventDefault()
    status.textContent = "Decrypting…"
    try {
      const response = await fetch(button.dataset.raw)
      if (!response.ok) throw new Error(response.statusText)
      const data = new Uint8Array(await response.arrayBuffer())
      const keyBytes = Uint8Array.from(atob(key.replace(/-/g, "+").replace(/_/g, "/")), (char) => char.charCodeAt(0))
      const cryptoKey = await crypto.subtle.importKey("raw", keyBytes, "AES-GCM", false, ["decrypt"])
      const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: data.slice(0, 12) }, cryptoKey, data.slice(12))
      const url = URL.createObjectURL(new Blob([plain]))
      const link = document.createElement("a")
      link.href = url
      link.download = button.dataset.filename
      link.click()
      setTimeout(() => URL.revokeObjectURL(url), 60000)
      status.textContent = ""
    } catch {
      status.textContent = "Could not decrypt. Check that the link is complete."
    }
  })
}
`

type Section = { title: string; rows: [command: string, description: string][] }

function usage(origin: string): Section[] {
  return [
    { title: "Install", rows: [["brew install stoffberg/tap/keep", ""]] },
    {
      title: "Use",
      rows: [
        ["keep report.pdf", "upload, prints the link"],
        ["keep -e .env", "encrypt first, the key stays in the link"],
        ["cat build.log | keep -n build.log", "upload stdin"],
        ["keep get <link>", "download, decrypting when needed"],
        ["keep ls", "uploads from this machine"],
        ["keep rm <link>", "delete"],
      ],
    },
    {
      title: "curl",
      rows: [
        [`curl -T report.pdf ${origin}`, "upload up to 100 MB, prints the link"],
        [`curl -OJ ${origin}/<id>`, "download"],
        ["curl -X DELETE -H 'Authorization: Bearer <token>' <link>", "delete, token from the X-Delete-Token upload header"],
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
  return lines.join("\n")
}

export function homePage(origin: string): string {
  const sections = usage(origin)
    .map((section) => {
      const rows = section.rows
        .map(([command, description]) => `<dt>${escapeHtml(command)}</dt><dd>${escapeHtml(description)}</dd>`)
        .join("")
      return `<h2>${section.title}</h2><dl>${rows}</dl>`
    })
    .join("")
  return layout(
    "keep",
    `<h1>${TAGLINE}</h1>
<p>No accounts, no expiry. Encryption happens on your machine and the key never reaches the server.</p>
${sections}
<footer><p><a href="${SOURCE_URL}">Source</a></p></footer>`,
  )
}

export function filePage(file: StoredFile, origin: string): string {
  const raw = `/${encodeURIComponent(file.id)}/raw`
  const details = [formatSize(file.size), file.uploaded.toISOString().slice(0, 10)]
  if (file.encrypted) {
    details.push("encrypted")
  }
  const data = `id="download" class="button" data-raw="${raw}" data-filename="${escapeHtml(file.filename)}"`
  const button = file.encrypted
    ? `<button ${data} disabled>Download</button>`
    : `<a ${data} href="${raw}" download>Download</a>`
  const status = file.encrypted ? "This link is missing the key that decrypts the file." : ""
  return layout(
    `${file.filename} · keep`,
    `<h1>${escapeHtml(file.filename)}</h1>
<p>${details.join(" · ")}</p>
${button}
<p id="status" role="status">${status}</p>
<h2>Terminal</h2>
<code id="command">keep get ${escapeHtml(`${origin}/${file.id}`)}</code>
<script>${DECRYPT_SCRIPT}</script>`,
  )
}

export function notFoundPage(): string {
  return layout(
    "Not found · keep",
    `<h1>Not found</h1>
<p>Nothing is kept at this address.</p>`,
  )
}

function layout(title: string, body: string): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>${escapeHtml(title)}</title>
<link rel="icon" href="${ICON}">
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
