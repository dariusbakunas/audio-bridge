import { useEffect, useRef } from "react";
import { apiUrl } from "../api";
import { LogEvent, MetadataEvent, OutputsResponse, QueueResponse } from "../types";

interface OutputsStreamOptions {
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
  onEvent: (items: QueueResponse["items"]) => void;
  onError: () => void;
}

export function useOutputsStream({ onEvent, onError }: OutputsStreamOptions) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onEventRef.current = onEvent;
    onErrorRef.current = onError;
  }, [onEvent, onError]);

  useEffect(() => {
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
  }, []);
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

export function useQueueStream({ onEvent, onError }: QueueStreamOptions) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    onEventRef.current = onEvent;
    onErrorRef.current = onError;
  }, [onEvent, onError]);

  useEffect(() => {
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
  }, []);
}
