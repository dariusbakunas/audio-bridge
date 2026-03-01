import { useEffect, useState } from "react";

import { apiUrl, fetchJson } from "../api";
import { TrackResolveResponse } from "../types";

export function useNowPlayingCover(nowPlayingTrackId: number | null | undefined) {
  const [nowPlayingCover, setNowPlayingCover] = useState<string | null>(null);
  const [nowPlayingCoverFailed, setNowPlayingCoverFailed] = useState<boolean>(false);
  const [nowPlayingAlbumId, setNowPlayingAlbumId] = useState<number | null>(null);

  useEffect(() => {
    if (!nowPlayingTrackId) {
      setNowPlayingCover(null);
      setNowPlayingCoverFailed(false);
      setNowPlayingAlbumId(null);
      return;
    }
    setNowPlayingCover(apiUrl(`/tracks/${nowPlayingTrackId}/cover`));
    setNowPlayingCoverFailed(false);
    let active = true;
    fetchJson<TrackResolveResponse>(`/tracks/resolve?track_id=${nowPlayingTrackId}`)
      .then((response) => {
        if (!active) return;
        setNowPlayingAlbumId(response?.album_id ?? null);
      })
      .catch(() => {
        if (!active) return;
        setNowPlayingAlbumId(null);
      });
    return () => {
      active = false;
    };
  }, [nowPlayingTrackId]);

  return {
    nowPlayingCover,
    nowPlayingCoverFailed,
    nowPlayingAlbumId,
    onCoverError: () => setNowPlayingCoverFailed(true)
  };
}
