import { useCallback } from "react";

import { AlbumProfileResponse } from "../types";

type UseAlbumModalActionsArgs = {
  albumViewId: number | null;
  setAlbumViewId: (value: number | null) => void;
  loadAlbumTracks: (albumId: number) => void;
  loadAlbums: () => void;
  setAlbumProfile: (value: AlbumProfileResponse | null) => void;
  loadCatalogProfiles: (albumId: number | null) => void;
};

export function useAlbumModalActions({
  albumViewId,
  setAlbumViewId,
  loadAlbumTracks,
  loadAlbums,
  setAlbumProfile,
  loadCatalogProfiles
}: UseAlbumModalActionsArgs) {
  const onSavedEdit = useCallback(() => {
    if (albumViewId !== null) {
      loadAlbumTracks(albumViewId);
    }
    loadAlbums();
  }, [albumViewId, loadAlbumTracks, loadAlbums]);

  const onUpdatedAlbumEdit = useCallback(
    (updatedAlbumId: number) => {
      if (albumViewId !== null) {
        setAlbumViewId(updatedAlbumId);
        loadAlbumTracks(updatedAlbumId);
      }
      loadAlbums();
    },
    [albumViewId, loadAlbumTracks, loadAlbums, setAlbumViewId]
  );

  const onCatalogUpdated = useCallback(
    ({ album }: { album?: AlbumProfileResponse | null }) => {
      if (album) {
        setAlbumProfile(album);
      } else {
        loadCatalogProfiles(albumViewId);
      }
    },
    [albumViewId, loadCatalogProfiles, setAlbumProfile]
  );

  return {
    onSavedEdit,
    onUpdatedAlbumEdit,
    onCatalogUpdated
  };
}
