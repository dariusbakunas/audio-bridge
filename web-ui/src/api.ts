type JsonValue = string | number | boolean | null | JsonObject | JsonValue[];
type JsonObject = { [key: string]: JsonValue };

// @ts-ignore
const DEFAULT_API_BASE = import.meta.env.VITE_API_BASE ?? "";
const API_BASE_STORAGE_KEY = "audioHub.apiBase";
const DEFAULT_FETCH_TIMEOUT_MS = 8000;

export function getStoredApiBase(): string {
  try {
    return localStorage.getItem(API_BASE_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

export function setStoredApiBase(value: string): void {
  const trimmed = value.trim();
  try {
    if (trimmed) {
      localStorage.setItem(API_BASE_STORAGE_KEY, trimmed);
    } else {
      localStorage.removeItem(API_BASE_STORAGE_KEY);
    }
  } catch {
    // Ignore storage failures (private mode, etc.)
  }
}

export function getDefaultApiBase(): string {
  return DEFAULT_API_BASE;
}

export function getEffectiveApiBase(): string {
  const stored = getStoredApiBase().trim();
  return stored || DEFAULT_API_BASE;
}

export function apiUrl(path: string): string {
  return `${getEffectiveApiBase()}${path}`;
}

export function apiWsUrl(path: string): string {
  const url = apiUrl(path);
  if (url.startsWith("http://")) {
    return url.replace("http://", "ws://");
  }
  if (url.startsWith("https://")) {
    return url.replace("https://", "wss://");
  }
  const scheme = window.location.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${window.location.host}${url}`;
}

function getApiBaseOrigin(): string {
  const base = getEffectiveApiBase().trim();
  const resolved = new URL(base || window.location.origin, window.location.origin);
  return resolved.origin;
}

export function safeMediaUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  let resolved: URL;
  try {
    const base = getEffectiveApiBase().trim();
    resolved = new URL(trimmed, base || window.location.origin);
  } catch {
    return null;
  }
  if (resolved.protocol !== "http:" && resolved.protocol !== "https:") {
    return null;
  }
  if (resolved.origin !== getApiBaseOrigin()) {
    return null;
  }
  return resolved.toString();
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const url = apiUrl(path);
  const controller = !init?.signal ? new AbortController() : null;
  const timeoutId = controller
    ? window.setTimeout(() => controller.abort(), DEFAULT_FETCH_TIMEOUT_MS)
    : null;
  let resp: Response;
  try {
    resp = await fetch(url, {
      ...init,
      signal: init?.signal ?? controller?.signal,
      headers: {
        "Content-Type": "application/json",
        ...(init?.headers || {})
      }
    });
  } catch (err) {
    if (timeoutId !== null) {
      window.clearTimeout(timeoutId);
    }
    if (err instanceof DOMException && err.name === "AbortError") {
      throw new Error(`Request timed out after ${DEFAULT_FETCH_TIMEOUT_MS}ms (${url}).`);
    }
    const base = getEffectiveApiBase();
    const target = base ? base : "current origin";
    const tlsHint = base.startsWith("https://")
      ? " If using HTTPS with a self-signed cert, trust it in Keychain or use mkcert."
      : "";
    throw new Error(`Network error connecting to ${target} (${url}).${tlsHint}`);
  }
  if (timeoutId !== null) {
    window.clearTimeout(timeoutId);
  }

  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(text || `${resp.status} ${resp.statusText}`);
  }

  if (resp.status === 204) {
    return null as T;
  }

  const text = await resp.text();
  if (!text.trim()) {
    return null as T;
  }

  const contentType = resp.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    try {
      return JSON.parse(text) as T;
    } catch {
      return null as T;
    }
  }

  return text as T;
}

export async function postJson<T>(path: string, body?: JsonObject): Promise<T> {
  return fetchJson<T>(path, {
    method: "POST",
    body: body ? JSON.stringify(body) : undefined
  });
}
