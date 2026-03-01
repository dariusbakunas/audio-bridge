import { useCallback, useEffect, useMemo, useState } from "react";

import {
  apiUrl,
  fetchJson,
  getDefaultApiBase,
  getEffectiveApiBase,
  getStoredApiBase,
  setStoredApiBase
} from "../api";

type UseHubConnectionArgs = {
  onHealthy?: () => void;
};

export function useHubConnection({ onHealthy }: UseHubConnectionArgs = {}) {
  const [serverConnected, setServerConnected] = useState<boolean>(false);
  const [serverConnecting, setServerConnecting] = useState<boolean>(true);
  const [serverError, setServerError] = useState<string | null>(null);
  const [apiBaseOverride, setApiBaseOverride] = useState<string>(() => getStoredApiBase());
  const apiBaseDefault = useMemo(() => getDefaultApiBase(), []);

  const handleApiBaseChange = useCallback((value: string) => {
    setApiBaseOverride(value);
    setStoredApiBase(value);
    setServerConnecting(true);
  }, []);

  const handleApiBaseReset = useCallback(() => {
    setApiBaseOverride("");
    setStoredApiBase("");
    setServerConnecting(true);
  }, []);

  const connectionError = useCallback((label: string, path?: string) => {
    const base = getEffectiveApiBase();
    const target = base ? base : "current origin";
    const tlsHint = base.startsWith("https://")
      ? " If using HTTPS with a self-signed cert, trust it in Keychain or use mkcert."
      : "";
    const url = path ? apiUrl(path) : null;
    const detail = url ? `${target} (${url})` : target;
    return `${label} (${detail}).${tlsHint}`;
  }, []);

  const markServerConnected = useCallback(() => {
    setServerConnected(true);
    setServerConnecting(false);
    setServerError(null);
  }, []);

  const markServerDisconnected = useCallback((message: string) => {
    setServerConnected(false);
    setServerConnecting(false);
    setServerError(message);
  }, []);

  useEffect(() => {
    let active = true;
    let timer: number | null = null;

    const checkHealth = async () => {
      try {
        await fetchJson<{ status: string }>("/health");
        if (!active) return;
        markServerConnected();
        onHealthy?.();
      } catch {
        if (!active) return;
        const message = connectionError("Hub server not reachable", "/health");
        markServerDisconnected(message);
      }
    };

    checkHealth();
    timer = window.setInterval(checkHealth, 5000);

    return () => {
      active = false;
      if (timer !== null) {
        window.clearInterval(timer);
      }
    };
  }, [apiBaseOverride, connectionError, markServerConnected, markServerDisconnected, onHealthy]);

  return {
    serverConnected,
    serverConnecting,
    serverError,
    apiBaseOverride,
    apiBaseDefault,
    handleApiBaseChange,
    handleApiBaseReset,
    connectionError,
    markServerConnected,
    markServerDisconnected
  };
}
