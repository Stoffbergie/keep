import { filePage, homePage, homeText, notFoundPage } from "./pages"
import { contentType, imageType, isText } from "./previews"

export interface Env {
  FILES: R2Bucket
}

export interface StoredFile {
  id: string
  filename: string
  contentType: string
  size: number
  uploaded: Date
  encrypted: boolean
  body?: ReadableStream
}

const ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"
const ID_LENGTH = 8
const TOKEN_LENGTH = 32

export default {
  async fetch(request, env) {
    try {
      return await route(request, env)
    } catch (error) {
      console.error(error)
      const message = error instanceof Error ? error.message : "Something went wrong"
      return text(`${message}\n`, 500)
    }
  },
} satisfies ExportedHandler<Env>

async function route(request: Request, env: Env): Promise<Response> {
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
    return download(request, env, url, path[0], wantsHtml(request))
  }
  if (method === "GET" && path.length === 2 && path[1] === "raw") {
    return download(request, env, url, path[0], false)
  }
  if (method === "GET" && path.length === 2 && path[1] === "preview") {
    return preview(request, env, path[0])
  }
  if (method === "DELETE" && path.length === 1) {
    return remove(request, env, path[0])
  }
  return notFound(request)
}

function home(request: Request, url: URL): Response {
  if (wantsHtml(request)) {
    return html(homePage(url.origin))
  }
  return text(homeText(url.origin))
}

async function upload(request: Request, env: Env, url: URL, filename: string): Promise<Response> {
  const id = await newId(env)
  const metadata = describe(request, filename)
  await env.FILES.put(id, await sizedBody(request), { customMetadata: metadata })
  return created(url, id, metadata.deleteToken)
}

async function multipart(request: Request, env: Env, url: URL, path: string[]): Promise<Response> {
  const uploadId = request.headers.get("x-upload-id") ?? ""

  if (request.method === "POST" && path.length === 0) {
    const id = await newId(env)
    const filename = url.searchParams.get("filename") || id
    const upload = await env.FILES.createMultipartUpload(id, {
      customMetadata: describe(request, filename),
    })
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
    return created(url, path[0], deleteToken)
  }

  return notFound(request)
}

async function download(
  request: Request,
  env: Env,
  url: URL,
  id: string,
  asPage: boolean,
): Promise<Response> {
  const file = await findFile(env, id, !asPage && request.method !== "HEAD")
  if (!file) {
    return notFound(request)
  }
  if (asPage) {
    let excerpt = ""
    if (!file.encrypted && file.size > 0 && isText(file)) {
      const object = await env.FILES.get(id, { range: { offset: 0, length: 1024 } })
      if (object) {
        excerpt = (await object.text()).replace(/\s+/g, " ").trim().slice(0, 180)
      }
    }
    return html(filePage(file, url.origin, excerpt))
  }
  return new Response(file.body, {
    headers: {
      "content-type": file.contentType,
      "content-length": String(file.size),
      "content-disposition": attachment(file.filename),
      "cache-control": "private, max-age=31536000, immutable",
      "x-content-type-options": "nosniff",
      "x-robots-tag": "noindex",
    },
  })
}

async function preview(request: Request, env: Env, id: string): Promise<Response> {
  const file = await findFile(env, id, false)
  const type = file && imageType(file)
  if (!file || !type) {
    return notFound(request)
  }
  const object = await env.FILES.get(id)
  if (!object) {
    return notFound(request)
  }
  return new Response(object.body, {
    headers: {
      "content-type": type,
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

async function findFile(env: Env, id: string, withBody: boolean): Promise<StoredFile | null> {
  if (withBody) {
    const object = await env.FILES.get(id)
    if (!object) {
      return null
    }
    return { ...fileFrom(id, object), body: object.body }
  }
  const object = await env.FILES.head(id)
  if (!object) {
    return null
  }
  return fileFrom(id, object)
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
  }
}

function describe(request: Request, filename: string): Record<string, string> {
  const metadata: Record<string, string> = {
    filename,
    contentType: contentType(filename, request.headers.get("content-type") ?? undefined),
    deleteToken: randomString(TOKEN_LENGTH),
  }
  if (request.headers.get("x-encrypted") === "true") {
    metadata.encrypted = "true"
  }
  return metadata
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

function created(url: URL, id: string, deleteToken: string): Response {
  const link = `${url.origin}/${id}`
  return new Response(`${link}\n`, {
    status: 201,
    headers: {
      "content-type": "text/plain; charset=utf-8",
      location: link,
      "x-delete-token": deleteToken,
    },
  })
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
