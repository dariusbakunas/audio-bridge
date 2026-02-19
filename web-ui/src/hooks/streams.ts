import { useEffect, useRef } from "react";
import { apiUrl } from "../api";
import { LogEvent, MetadataEvent, OutputsResponse, QueueResponse, StatusResponse } from "../types";

interface OutputsStreamOptions {
  enabled?: boolean;
  sourceKey?: string;
  onEvent: (data: OutputsResponse) => void;
  onError: () => void;
}

interface MetadataStreamOptions {
  enabled: boolean;
  onEvent: (event: MetadataEvent) => void;
  onError: () => void;
}

interface LogsStreamOptions {
  enabled: boolean;
  onSnapshot: (items: LogEvent[]) => void;
  onEvent: (event: LogEvent) => void;
  onError: () => void;
}

interface QueueStreamOptions {
  enabled?: boolean;
  sourceKey?: string;
  onEvent: (items: QueueResponse["items"]) => void;
  onError: () => void;
}

interface StatusStreamOptions {
  enabled?: boolean;
  sourceKey?: string;
  onEvent: (data: StatusResponse) => void;
  onError: () => void;
}

export function useOutputsStream({ enabled = true, sourceKey, onEvent, onError }: OutputsStreamOptions) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onEventRef.current = onEvent;
    onErrorRef.current = onError;
  }, [onEvent, onError]);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/outputs/stream"));
    stream.addEventListener("outputs", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as OutputsResponse;
      onEventRef.current(data);
    });
    stream.onerror = () => {
      if (!mounted) return;
      onErrorRef.current();
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [enabled, sourceKey]);
}

export function useMetadataStream({ enabled, onEvent, onError }: MetadataStreamOptions) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onEventRef.current = onEvent;
    onErrorRef.current = onError;
  }, [onEvent, onError]);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/metadata/stream"));
    stream.addEventListener("metadata", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as MetadataEvent;
      onEventRef.current(data);
    });
    stream.onerror = () => {
      if (!mounted) return;
      onErrorRef.current();
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [enabled]);
}

export function useLogsStream({ enabled, onSnapshot, onEvent, onError }: LogsStreamOptions) {
  const onSnapshotRef = useRef(onSnapshot);
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onSnapshotRef.current = onSnapshot;
    onEventRef.current = onEvent;
    onErrorRef.current = onError;
  }, [onSnapshot, onEvent, onError]);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/logs/stream"));
    stream.addEventListener("logs", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as LogEvent[];
      onSnapshotRef.current(data);
    });
    stream.addEventListener("log", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as LogEvent;
      onEventRef.current(data);
    });
    stream.onerror = () => {
      if (!mounted) return;
      onErrorRef.current();
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [enabled]);
}

export function useQueueStream({ enabled = true, sourceKey, onEvent, onError }: QueueStreamOptions) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onEventRef.current = onEvent;
    onErrorRef.current = onError;
  }, [onEvent, onError]);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/queue/stream"));
    stream.addEventListener("queue", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as QueueResponse;
      onEventRef.current(data.items);
    });
    stream.onerror = () => {
      if (!mounted) return;
      onErrorRef.current();
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [enabled, sourceKey]);
}

export function useStatusStream({ enabled = true, sourceKey, onEvent, onError }: StatusStreamOptions) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onEventRef.current = onEvent;
    onErrorRef.current = onError;
  }, [onEvent, onError]);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/status/stream"));
    stream.addEventListener("status", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as StatusResponse;
      onEventRef.current(data);
    });
    stream.onerror = () => {
      if (!mounted) return;
      onErrorRef.current();
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [enabled, sourceKey]);
}
