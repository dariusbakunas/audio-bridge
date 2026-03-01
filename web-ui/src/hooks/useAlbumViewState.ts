import { useMemo } from "react";

import { AlbumSummary } from "../types";
import { normalizeMatch } from "../utils/viewFormatters";

type UseAlbumViewStateArgs = {
  albums: AlbumSummary[];
  albumViewId: number | null;
  albumSearch: string;
  statusAlbum?: string | null;
  statusArtist?: string | null;
  nowPlayingAlbumId: number | null;
};

export function useAlbumViewState({
  albums,
  albumViewId,
  albumSearch,
  statusAlbum,
  statusArtist,
  nowPlayingAlbumId
}: UseAlbumViewStateArgs) {
  const selectedAlbum = useMemo(
    () => albums.find((album) => album.id === albumViewId) ?? null,
    [albums, albumViewId]
  );

  const filteredAlbums = useMemo(() => {
    const query = albumSearch.trim().toLowerCase();
    if (!query) return albums;
    return albums.filter((album) => {
      const title = album.title?.toLowerCase() ?? "";
      const artist = album.artist?.toLowerCase() ?? "";
      const year = album.year ? String(album.year) : "";
      const originalYear = album.original_year ? String(album.original_year) : "";
      const editionYear = album.edition_year ? String(album.edition_year) : "";
      const editionLabel = album.edition_label?.toLowerCase() ?? "";
      return (
        title.includes(query) ||
        artist.includes(query) ||
        year.includes(query) ||
        originalYear.includes(query) ||
        editionYear.includes(query) ||
        editionLabel.includes(query)
      );
    });
  }, [albums, albumSearch]);

  const heuristicAlbumId = useMemo(() => {
    const albumKey = normalizeMatch(statusAlbum);
    if (!albumKey) return null;
    const artistKey = normalizeMatch(statusArtist);
    const allowArtistMismatch = (albumArtist?: string | null) => {
      if (!albumArtist) return true;
      const key = normalizeMatch(albumArtist);
      return key === "various artists" || key === "various" || key === "va";
    };
    const match = albums.find((album) => {
      if (normalizeMatch(album.title) !== albumKey) return false;
      if (!artistKey) return true;
      if (!album.artist) return true;
      if (normalizeMatch(album.artist) === artistKey) return true;
      return allowArtistMismatch(album.artist);
    });
    return match?.id ?? null;
  }, [albums, statusAlbum, statusArtist]);

  const activeAlbumId = nowPlayingAlbumId ?? heuristicAlbumId;

  return {
    selectedAlbum,
    filteredAlbums,
    activeAlbumId
  };
}
