import { useCallback, useMemo, useState } from "react";

import { AlbumSummary, TrackSummary } from "../types";

type MatchTarget = {
  trackId?: number;
  title: string;
  artist: string;
  album?: string | null;
};

type EditTarget = {
  trackId?: number;
  label: string;
  defaults: {
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    albumArtist?: string | null;
    year?: number | null;
    trackNumber?: number | null;
    discNumber?: number | null;
  };
};

type AlbumEditTarget = {
  albumId: number;
  label: string;
  artist: string;
  defaults: {
    title?: string | null;
    albumArtist?: string | null;
    year?: number | null;
  };
};

type UseAlbumMetadataTargetsArgs = {
  albumTracks: TrackSummary[];
  selectedAlbum: AlbumSummary | null;
};

export function useAlbumMetadataTargets({ albumTracks, selectedAlbum }: UseAlbumMetadataTargetsArgs) {
  const [matchTarget, setMatchTarget] = useState<MatchTarget | null>(null);
  const [editTarget, setEditTarget] = useState<EditTarget | null>(null);
  const [albumEditTarget, setAlbumEditTarget] = useState<AlbumEditTarget | null>(null);

  const openTrackMatchForAlbum = useCallback(
    (trackId: number) => {
      const track = albumTracks.find((item) => item.id === trackId);
      const title = track?.title ?? track?.file_name ?? "Unknown track";
      const artist = track?.artist ?? "Unknown artist";
      const album = track?.album ?? selectedAlbum?.title ?? "";
      setMatchTarget({
        trackId: track?.id ?? trackId,
        title,
        artist,
        album
      });
    },
    [albumTracks, selectedAlbum]
  );

  const openAlbumEditor = useCallback(() => {
    if (!selectedAlbum) return;
    const label = selectedAlbum.artist
      ? `${selectedAlbum.title} — ${selectedAlbum.artist}`
      : selectedAlbum.title;
    setAlbumEditTarget({
      albumId: selectedAlbum.id,
      label,
      artist: selectedAlbum.artist ?? "Unknown artist",
      defaults: {
        title: selectedAlbum.title,
        albumArtist: selectedAlbum.artist ?? null,
        year: selectedAlbum.year ?? null
      }
    });
  }, [selectedAlbum]);

  const openTrackEditorForAlbum = useCallback(
    (trackId: number) => {
      const track = albumTracks.find((item) => item.id === trackId);
      const title = track?.title ?? track?.file_name ?? "Unknown track";
      const artist = track?.artist ?? "";
      const album = track?.album ?? selectedAlbum?.title ?? "";
      const label = artist ? `${title} — ${artist}` : title;
      setEditTarget({
        trackId: track?.id ?? trackId,
        label,
        defaults: {
          title,
          artist,
          album,
          albumArtist: selectedAlbum?.artist ?? null,
          trackNumber: track?.track_number ?? null,
          discNumber: track?.disc_number ?? null
        }
      });
    },
    [albumTracks, selectedAlbum]
  );

  const matchLabel = useMemo(
    () => (matchTarget ? `${matchTarget.title}${matchTarget.artist ? ` — ${matchTarget.artist}` : ""}` : ""),
    [matchTarget]
  );
  const matchDefaults = useMemo(
    () =>
      matchTarget
        ? {
            title: matchTarget.title,
            artist: matchTarget.artist,
            album: matchTarget.album ?? ""
          }
        : { title: "", artist: "", album: "" },
    [matchTarget]
  );

  return {
    matchTarget,
    setMatchTarget,
    editTarget,
    setEditTarget,
    albumEditTarget,
    setAlbumEditTarget,
    openTrackMatchForAlbum,
    openAlbumEditor,
    openTrackEditorForAlbum,
    matchLabel,
    matchDefaults,
    editLabel: editTarget?.label ?? "",
    editDefaults: editTarget?.defaults ?? {},
    albumEditLabel: albumEditTarget?.label ?? "",
    albumEditDefaults: albumEditTarget?.defaults ?? {}
  };
}
