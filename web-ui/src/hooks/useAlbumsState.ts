import { useCallback, useEffect, useRef, useState } from "react";

import { apiUrl, fetchJson } from "../api";
import {
  AlbumListResponse,
  AlbumProfileResponse,
  AlbumSummary,
  TrackListResponse,
  TrackSummary
} from "../types";

type UseAlbumsStateArgs = {
  serverConnected: boolean;
  streamKey: string;
  albumViewId: number | null;
  connectionError: (label: string, path?: string) => string;
  markServerConnected: () => void;
};

export function useAlbumsState({
  serverConnected,
  streamKey,
  albumViewId,
  connectionError,
  markServerConnected
}: UseAlbumsStateArgs) {
  const [albums, setAlbums] = useState<AlbumSummary[]>([]);
  const [albumsLoading, setAlbumsLoading] = useState<boolean>(false);
  const [albumsError, setAlbumsError] = useState<string | null>(null);
  const [albumTracks, setAlbumTracks] = useState<TrackSummary[]>([]);
  const [albumTracksLoading, setAlbumTracksLoading] = useState<boolean>(false);
  const [albumTracksError, setAlbumTracksError] = useState<string | null>(null);
  const [albumProfile, setAlbumProfile] = useState<AlbumProfileResponse | null>(null);
  const [catalogLoading, setCatalogLoading] = useState<boolean>(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);

  const albumsReloadTimerRef = useRef<number | null>(null);
  const albumsReloadQueuedRef = useRef(false);
  const albumsLoadingRef = useRef(false);

  const loadAlbums = useCallback(async () => {
    if (!albumsLoadingRef.current) {
      setAlbumsLoading(true);
    }
    albumsLoadingRef.current = true;
    try {
      const response = await fetchJson<AlbumListResponse>("/albums?limit=200");
      setAlbums(response.items ?? []);
      setAlbumsError(null);
      markServerConnected();
    } catch (err) {
      setAlbumsError((err as Error).message);
    } finally {
      albumsLoadingRef.current = false;
      setAlbumsLoading(false);
      if (albumsReloadQueuedRef.current) {
        albumsReloadQueuedRef.current = false;
        if (albumsReloadTimerRef.current === null) {
          albumsReloadTimerRef.current = window.setTimeout(() => {
            albumsReloadTimerRef.current = null;
            loadAlbums();
          }, 250);
        }
      }
    }
  }, [markServerConnected]);

  const requestAlbumsReload = useCallback(() => {
    if (albumsLoadingRef.current) {
      albumsReloadQueuedRef.current = true;
      return;
    }
    if (albumsReloadTimerRef.current !== null) return;
    albumsReloadTimerRef.current = window.setTimeout(() => {
      albumsReloadTimerRef.current = null;
      loadAlbums();
    }, 250);
  }, [loadAlbums]);

  useEffect(() => {
    if (!serverConnected) return;
    loadAlbums();
  }, [loadAlbums, requestAlbumsReload, serverConnected]);

  useEffect(() => {
    if (!serverConnected) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/albums/stream"));
    stream.addEventListener("albums", () => {
      if (!mounted) return;
      requestAlbumsReload();
    });
    stream.onerror = () => {
      if (!mounted) return;
      const message = connectionError("Live albums disconnected", "/albums/stream");
      setAlbumsError(message);
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [connectionError, requestAlbumsReload, serverConnected, streamKey]);

  const loadAlbumTracks = useCallback(
    async (id: number | null) => {
      if (id === null) return;
      setAlbumTracksLoading(true);
      try {
        const response = await fetchJson<TrackListResponse>(`/tracks?album_id=${id}&limit=500`);
        setAlbumTracks(response.items ?? []);
        setAlbumTracksError(null);
        markServerConnected();
      } catch (err) {
        setAlbumTracksError((err as Error).message);
      } finally {
        setAlbumTracksLoading(false);
      }
    },
    [markServerConnected]
  );

  const loadCatalogProfiles = useCallback(async (id: number | null) => {
    if (id === null) {
      setAlbumProfile(null);
      setCatalogError(null);
      return;
    }
    setCatalogError(null);
    setCatalogLoading(true);
    try {
      const albumPromise = fetchJson<AlbumProfileResponse>(`/albums/profile?album_id=${id}&lang=en-US`);
      const [albumResult] = await Promise.allSettled([albumPromise]);
      if (albumResult.status === "fulfilled") {
        setAlbumProfile(albumResult.value);
      } else {
        setCatalogError(
          albumResult.reason instanceof Error ? albumResult.reason.message : String(albumResult.reason)
        );
      }
    } catch (err) {
      setCatalogError((err as Error).message);
    } finally {
      setCatalogLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!serverConnected) return;
    loadAlbumTracks(albumViewId);
  }, [albumViewId, loadAlbumTracks, serverConnected]);

  useEffect(() => {
    if (!serverConnected) return;
    loadCatalogProfiles(albumViewId);
  }, [albumViewId, loadCatalogProfiles, serverConnected]);

  return {
    albums,
    albumsLoading,
    albumsError,
    setAlbumsError,
    albumTracks,
    albumTracksLoading,
    albumTracksError,
    setAlbumTracksError,
    albumProfile,
    setAlbumProfile,
    catalogLoading,
    catalogError,
    loadAlbums,
    loadAlbumTracks,
    loadCatalogProfiles
  };
}
