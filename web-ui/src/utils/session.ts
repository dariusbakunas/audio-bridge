export const WEB_DEFAULT_SESSION_NAME = "Default";

export function isDefaultSessionName(name: string | null | undefined): boolean {
  return (name ?? "").trim().toLowerCase() === WEB_DEFAULT_SESSION_NAME.toLowerCase();
}

export function getOrCreateWebSessionClientId(storageKey: string): string {
  try {
    const existing = localStorage.getItem(storageKey);
    if (existing) return existing;
    const generated =
      typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    localStorage.setItem(storageKey, generated);
    return generated;
  } catch {
    return `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
  }
}
