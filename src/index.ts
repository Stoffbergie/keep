import { filePage, homePage, homeText, notFoundPage } from "./pages"
import { contentType, imageType, isText } from "./previews"

export interface Env {
  FILES: R2Bucket
  LOOKUPS: RateLimit
}

export interface StoredFile {
  id: string
  filename: string
  contentType: string
  size: number
  uploaded: Date
  encrypted: boolean
  once: boolean
  encryptedName?: string
  expires?: Date
  body?: ReadableStream
  range?: { offset: number; length: number }
}

const ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"
const ID_LENGTH = 8
const TOKEN_LENGTH = 32
const EXPIRY_UNITS: Record<string, number> = { m: 60_000, h: 3_600_000, d: 86_400_000 }

export default {
  async fetch(request, env, ctx) {
    try {
      return await route(request, env, ctx)
    } catch (error) {
      console.error(error)
      const message = error instanceof Error ? error.message : "Something went wrong"
      return text(`${message}\n`, 500)
    }
  },
  async scheduled(_controller, env) {
    await deleteExpired(env)
  },
} satisfies ExportedHandler<Env>

async function route(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url)
  const path = url.pathname.split("/").filter(Boolean).map(decodeURIComponent)
  const method = request.method === "HEAD" ? "GET" : request.method

  if (path[0] === "api" && path[1] === "multipart") {
    return multipart(request, env, url, path.slice(2))
  }
  if (method === "GET" && path.length === 0) {
    return home(request, url)
  }
  if (method === "GET" && path.length === 1 && path[0] === "health") {
    return Response.json({ ok: true })
  }
  if (method === "GET" && path.length === 1 && path[0] === "robots.txt") {
    return text(`User-agent: *\nAllow: /\nSitemap: ${url.origin}/sitemap.xml\n`)
  }
  if (method === "GET" && path.length === 1 && path[0] === "sitemap.xml") {
    return new Response(`<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>${url.origin}/</loc></url></urlset>`, {
      headers: { "content-type": "application/xml; charset=utf-8" },
    })
  }
  if (method === "PUT" && path.length === 1) {
    return upload(request, env, url, path[0])
  }
  if (method === "GET" && path.length === 1) {
    return lookup(request, env, () => download(request, env, ctx, url, path[0], wantsHtml(request)))
  }
  if (method === "GET" && path.length === 2 && path[1] === "raw") {
    return lookup(request, env, () => download(request, env, ctx, url, path[0], false))
  }
  if (method === "GET" && path.length === 2 && path[1] === "preview") {
    return lookup(request, env, () => preview(request, env, path[0]))
  }
  if (method === "DELETE" && path.length === 1) {
    return lookup(request, env, () => remove(request, env, path[0]))
  }
  return notFound(request)
}

async function lookup(request: Request, env: Env, find: () => Promise<Response>): Promise<Response> {
  const address = request.headers.get("cf-connecting-ip")
  if (address) {
    const { success } = await env.LOOKUPS.limit({ key: address })
    if (!success) {
      return new Response("Too many requests. Try again in a minute.\n", {
        status: 429,
        headers: { "content-type": "text/plain; charset=utf-8", "retry-after": "60" },
      })
    }
  }
  return find()
}

function home(request: Request, url: URL): Response {
  if (wantsHtml(request)) {
    return html(homePage(url.origin))
  }
  return text(homeText(url.origin))
}

async function upload(request: Request, env: Env, url: URL, filename: string): Promise<Response> {
  const metadata = describe(request, filename)
  if (!metadata) {
    return invalidExpiry()
  }
  const id = randomString(ID_LENGTH)
  const stored = await env.FILES.put(id, await sizedBody(request), {
    customMetadata: metadata,
    onlyIf: new Headers({ "if-none-match": "*" }),
  })
  if (!stored) {
    return text("That code was just taken, upload again\n", 503)
  }
  return created(url, id, metadata.deleteToken, metadata.expiresAt)
}

async function multipart(request: Request, env: Env, url: URL, path: string[]): Promise<Response> {
  const uploadId = request.headers.get("x-upload-id") ?? ""

  if (request.method === "POST" && path.length === 0) {
    const id = await newId(env)
    const filename = url.searchParams.get("filename") || id
    const metadata = describe(request, filename)
    if (!metadata) {
      return invalidExpiry()
    }
    const upload = await env.FILES.createMultipartUpload(id, { customMetadata: metadata })
    return Response.json({ id, uploadId: upload.uploadId })
  }

  if (request.method === "PUT" && path.length === 2) {
    const upload = env.FILES.resumeMultipartUpload(path[0], uploadId)
    const part = await upload.uploadPart(Number(path[1]), await sizedBody(request))
    return Response.json(part)
  }

  if (request.method === "POST" && path.length === 1) {
    const { parts } = await request.json<{ parts: R2UploadedPart[] }>()
    const upload = env.FILES.resumeMultipartUpload(path[0], uploadId)
    await upload.complete(parts)
    const object = await env.FILES.head(path[0])
    const deleteToken = object?.customMetadata?.deleteToken
    if (!deleteToken) {
      return text("Upload delete token is missing\n", 500)
    }
    return created(url, path[0], deleteToken, object?.customMetadata?.expiresAt)
  }

  return notFound(request)
}

async function download(
  request: Request,
  env: Env,
  ctx: ExecutionContext,
  url: URL,
  id: string,
  asPage: boolean,
): Promise<Response> {
  const range = asPage ? null : request.headers.get("range")
  let file: StoredFile | null
  try {
    file = await findFile(env, id, !asPage && request.method !== "HEAD", range ? request.headers : undefined)
  } catch (error) {
    if (range && error instanceof Error && error.message.includes("not satisfiable")) {
      return text("Range not satisfiable\n", 416)
    }
    throw error
  }
  if (!file) {
    return notFound(request)
  }
  if (asPage) {
    let excerpt = ""
    if (!file.encrypted && !file.once && file.size > 0 && isText(file)) {
      const object = await env.FILES.get(id, { range: { offset: 0, length: 1024 } })
      if (object) {
        excerpt = (await object.text()).replace(/\s+/g, " ").trim().slice(0, 180)
      }
    }
    return html(filePage(file, url.origin, excerpt))
  }
  const headers = new Headers({
    "content-type": file.contentType,
    "content-length": String(file.size),
    "content-disposition": attachment(file.filename),
    "cache-control": "private, max-age=31536000, immutable",
    "x-content-type-options": "nosniff",
    "x-robots-tag": "noindex",
    "accept-ranges": "bytes",
  })
  if (file.encryptedName) {
    headers.set("x-encrypted-name", file.encryptedName)
  }
  let status = 200
  let length = file.size
  if (file.range) {
    const end = file.range.offset + file.range.length - 1
    status = 206
    length = file.range.length
    headers.set("content-length", String(length))
    headers.set("content-range", `bytes ${file.range.offset}-${end}/${file.size}`)
  }
  let body = file.body
  if (file.once && file.body) {
    const stream = new FixedLengthStream(length)
    const streamed = file.body.pipeTo(stream.writable).catch(() => undefined)
    ctx.waitUntil(streamed.then(() => env.FILES.delete(id)))
    body = stream.readable
  }
  return new Response(body, { status, headers })
}

async function preview(request: Request, env: Env, id: string): Promise<Response> {
  const file = await findFile(env, id, request.method !== "HEAD")
  const type = file && imageType(file)
  if (!file || !type) {
    await file?.body?.cancel()
    return notFound(request)
  }
  return new Response(file.body, {
    headers: {
      "content-type": type,
      "content-length": String(file.size),
      "content-disposition": attachment(file.filename).replace(/^attachment/, "inline"),
      "cache-control": "private, max-age=300",
      "x-content-type-options": "nosniff",
      "x-robots-tag": "noindex",
    },
  })
}

async function remove(request: Request, env: Env, id: string): Promise<Response> {
  const object = await env.FILES.head(id)
  if (!object) {
    return text("Not found\n", 404)
  }
  const token = (request.headers.get("authorization") ?? "").replace(/^Bearer\s+/i, "")
  if (!sameToken(token, object.customMetadata?.deleteToken)) {
    return text("Wrong delete token\n", 403)
  }
  await env.FILES.delete(id)
  return new Response(null, { status: 204 })
}

async function findFile(env: Env, id: string, withBody: boolean, range?: Headers): Promise<StoredFile | null> {
  if (withBody) {
    const object = await env.FILES.get(id, { range })
    if (!object) {
      return null
    }
    if (isExpired(object)) {
      await object.body.cancel()
      return null
    }
    return { ...fileFrom(id, object), body: object.body, range: range ? byteRange(object) : undefined }
  }
  const object = await env.FILES.head(id)
  if (!object || isExpired(object)) {
    return null
  }
  return fileFrom(id, object)
}

function byteRange(object: R2ObjectBody): { offset: number; length: number } | undefined {
  const range = object.range
  if (!range) {
    return undefined
  }
  if ("suffix" in range) {
    const length = Math.min(range.suffix, object.size)
    return { offset: object.size - length, length }
  }
  const offset = range.offset ?? 0
  return { offset, length: range.length ?? object.size - offset }
}

async function deleteExpired(env: Env): Promise<void> {
  let cursor: string | undefined
  do {
    const listed = await env.FILES.list({ cursor, include: ["customMetadata"] })
    const expired = listed.objects.filter(isExpired).map((object) => object.key)
    if (expired.length > 0) {
      await env.FILES.delete(expired)
    }
    cursor = listed.truncated ? listed.cursor : undefined
  } while (cursor)
}

function isExpired(object: R2Object): boolean {
  const expiresAt = object.customMetadata?.expiresAt
  return expiresAt !== undefined && new Date(expiresAt) <= new Date()
}

function fileFrom(id: string, object: R2Object): StoredFile {
  const metadata = object.customMetadata ?? {}
  let uploaded = object.uploaded
  if (metadata.createdAt) {
    uploaded = new Date(metadata.createdAt)
  }
  return {
    id,
    filename: metadata.filename || id,
    contentType: contentType(metadata.filename || id, metadata.contentType),
    size: object.size,
    uploaded,
    encrypted: metadata.encrypted === "true",
    once: metadata.once === "true",
    encryptedName: metadata.encryptedName,
    expires: metadata.expiresAt ? new Date(metadata.expiresAt) : undefined,
  }
}

function describe(request: Request, filename: string): Record<string, string> | null {
  const metadata: Record<string, string> = {
    filename,
    contentType: contentType(filename, request.headers.get("content-type") ?? undefined),
    deleteToken: randomString(TOKEN_LENGTH),
  }
  if (request.headers.get("x-encrypted") === "true") {
    metadata.encrypted = "true"
    const encryptedName = request.headers.get("x-encrypted-name")
    if (encryptedName) {
      metadata.encryptedName = encryptedName
    }
  }
  const expires = request.headers.get("x-expires")?.trim()
  if (expires === "once") {
    metadata.once = "true"
  } else if (expires !== undefined) {
    const match = expires.match(/^([1-9]\d{0,5})([mhd])$/)
    if (!match) {
      return null
    }
    metadata.expiresAt = new Date(Date.now() + Number(match[1]) * EXPIRY_UNITS[match[2]]).toISOString()
  }
  return metadata
}

function invalidExpiry(): Response {
  return text("Expiry must be a number of minutes, hours or days, like 30m, 12h or 7d, or once\n", 400)
}

async function sizedBody(request: Request): Promise<ReadableStream | ArrayBuffer> {
  if (request.body && request.headers.has("content-length")) {
    return request.body
  }
  return request.arrayBuffer()
}

async function newId(env: Env): Promise<string> {
  while (true) {
    const id = randomString(ID_LENGTH)
    const existing = await env.FILES.head(id)
    if (!existing) {
      return id
    }
  }
}

function randomString(length: number): string {
  const bytes = crypto.getRandomValues(new Uint8Array(length))
  return Array.from(bytes, (byte) => ALPHABET[byte % ALPHABET.length]).join("")
}

function sameToken(given: string, expected: string | undefined): boolean {
  if (!expected || given.length !== expected.length) {
    return false
  }
  const encoder = new TextEncoder()
  return crypto.subtle.timingSafeEqual(encoder.encode(given), encoder.encode(expected))
}

function created(url: URL, id: string, deleteToken: string, expiresAt?: string): Response {
  const link = `${url.origin}/${id}`
  const headers = new Headers({
    "content-type": "text/plain; charset=utf-8",
    location: link,
    "x-delete-token": deleteToken,
  })
  if (expiresAt) {
    headers.set("x-expires-at", expiresAt)
  }
  return new Response(`${link}\n`, { status: 201, headers })
}

function attachment(filename: string): string {
  const fallback = filename.replace(/[^\x20-\x7e]|["\\]/g, "_")
  const encoded = encodeURIComponent(filename).replace(/['()*]/g, (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`)
  return `attachment; filename="${fallback}"; filename*=UTF-8''${encoded}`
}

function wantsHtml(request: Request): boolean {
  const crawler = /WhatsApp|facebookexternalhit|Facebot|meta-externalagent|meta-externalfetcher|Twitterbot|LinkedInBot|Slackbot|Discordbot|TelegramBot|Googlebot|bingbot/i
  return (request.headers.get("accept") ?? "").includes("text/html") || crawler.test(request.headers.get("user-agent") ?? "")
}

function notFound(request: Request): Response {
  if (wantsHtml(request)) {
    return html(notFoundPage(), 404)
  }
  return text("Not found\n", 404)
}

function text(body: string, status = 200): Response {
  return new Response(body, {
    status,
    headers: { "content-type": "text/plain; charset=utf-8" },
  })
}

function html(body: string, status = 200): Response {
  return new Response(body, {
    status,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "content-security-policy":
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'",
      "referrer-policy": "no-referrer",
      "x-content-type-options": "nosniff",
    },
  })
}
