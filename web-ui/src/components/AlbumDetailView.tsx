import { useEffect, useRef } from "react";
import { AlbumProfileResponse, AlbumSummary, TrackSummary } from "../types";
import hiResBadge from "../assets/hi-res.png";
import TrackMenu from "./TrackMenu";
import { BookOpen, Heart, Pause, Pencil, Play, Share2 } from "lucide-react";

interface AlbumDetailViewProps {
  album: AlbumSummary | null;
  tracks: TrackSummary[];
  loading: boolean;
  error: string | null;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  formatMs: (ms?: number | null) => string;
  activeAlbumId: number | null;
  isPlaying: boolean;
  isPaused: boolean;
  onPause: () => void;
  onPlayAlbum: () => void;
  onPlayTrack: (track: TrackSummary) => void;
  trackMenuPath: string | null;
  trackMenuPosition: { top: number; right: number; up: boolean } | null;
  onToggleMenu: (path: string, target: Element) => void;
  onMenuPlay: (path: string) => void;
  onMenuQueue: (path: string) => void;
  onMenuPlayNext: (path: string) => void;
  onMenuRescan: (path: string) => void;
  onFixTrackMatch: (path: string) => void;
  onEditTrackMetadata: (path: string) => void;
  onAnalyzeTrack: (track: TrackSummary) => void;
  onEditAlbumMetadata: () => void;
  onEditCatalogMetadata: () => void;
  onReadAlbumNotes: () => void;
  albumProfile?: AlbumProfileResponse | null;
  nowPlayingPath?: string | null;
}

export default function AlbumDetailView({
  album,
  tracks,
  loading,
  error,
  placeholder,
  canPlay,
  formatMs,
  activeAlbumId,
  isPlaying,
  isPaused,
  onPause,
  onPlayAlbum,
  onPlayTrack,
  trackMenuPath,
  trackMenuPosition,
  onToggleMenu,
  onMenuPlay,
  onMenuQueue,
  onMenuPlayNext,
  onMenuRescan,
  onFixTrackMatch,
  onEditTrackMetadata,
  onAnalyzeTrack,
  onEditAlbumMetadata,
  onEditCatalogMetadata,
  onReadAlbumNotes,
  albumProfile,
  nowPlayingPath
}: AlbumDetailViewProps) {
  const heroRef = useRef<HTMLDivElement | null>(null);
  const isActive = Boolean(album?.id && activeAlbumId === album.id && (isPlaying || isPaused));
  const isActivePlaying = Boolean(album?.id && activeAlbumId === album.id && isPlaying);
  const totalDuration = tracks.reduce((sum, track) => sum + (track.duration_ms ?? 0), 0);
  const albumNotes = albumProfile?.notes?.text?.trim() ?? "";
  const hasMultipleDiscs =
    new Set(
      tracks
        .map((track) => track.disc_number)
        .filter((disc): disc is number => disc !== null && disc !== undefined)
    ).size > 1;
  const displayYear = album?.original_year ?? album?.year ?? null;
  const editionLabel = album?.edition_label?.trim() ?? "";
  const editionYear = album?.edition_year ?? null;
  const editionDetail = editionLabel || editionYear
    ? `${editionLabel || "Edition"}${editionYear ? ` (${editionYear})` : ""}`
    : "";
  useEffect(() => {
    if (!heroRef.current) return;
    heroRef.current.scrollIntoView({ block: "start", behavior: "smooth" });
  }, [album?.id]);
  const formatSampleRate = (hz?: number | null) => {
    if (!hz) return null;
    const khz = hz / 1000;
    const value = Number.isInteger(khz) ? khz.toFixed(0) : khz.toFixed(1);
    return `${value}kHz`;
  };
  const formatTrackQuality = (track: TrackSummary) => {
    const format = track.format ?? "—";
    const rate = formatSampleRate(track.sample_rate);
    const depth = track.bit_depth ? `${track.bit_depth}bit` : null;
    const detail = rate && depth ? `${rate}/${depth}` : rate ?? depth;
    return detail ? `${format} ${detail}` : format;
  };
  return (
    <section className="album-view">
      <div className="album-hero" ref={heroRef}>
        <div className="album-hero-inner">
          <div className="album-detail-left">
            <div className="album-cover-frame large">
              <img
                className="album-cover large"
                src={
                  album?.cover_art_url
                    ? `${album.cover_art_url}${album.cover_art_path ? `?v=${encodeURIComponent(album.cover_art_path)}` : ""}`
                    : placeholder(album?.title, album?.artist)
                }
                alt={album?.title ?? "Album art"}
              />
            </div>
          </div>
          <div className="album-detail-right">
            <div className="album-meta">
              <div className="album-eyebrow">Album</div>
              <h2 className="album-detail-title">
                <span className="album-detail-title-text">
                  {album?.title ?? "Unknown album"} {editionLabel ? (
                    <span className="album-edition-label">{editionLabel}</span>
                ) : null}
                </span>
                {album?.hi_res ? (
                  <span
                    className="hires-badge detail"
                    style={{ backgroundImage: `url(${hiResBadge})` }}
                    role="img"
                    aria-label="Hi-res audio"
                    title="Hi-res audio (24-bit)"
                  />
                ) : null}
              </h2>
              <div className="album-meta-line">
                <span>{album?.artist ?? "Unknown artist"}</span>
                {displayYear ? <span className="meta-sep">•</span> : null}
                {displayYear ? <span>{displayYear}</span> : null}
                <span className="meta-sep">•</span>
                <span>{album?.track_count ?? tracks.length} tracks</span>
              </div>
              <div className="album-meta-sub mono">Total length: {formatMs(totalDuration)}</div>
              <div className="album-actions">
                <button
                  className="btn album-play-cta"
                  type="button"
                  onClick={() => {
                    if (isActive) {
                      onPause();
                      return;
                    }
                    onPlayAlbum();
                  }}
                  disabled={!canPlay}
                  aria-label={isActive ? (isPaused ? "Resume playback" : "Pause playback") : "Play album"}
                  title={isActive ? (isPaused ? "Resume" : "Pause") : "Play album"}
                >
                  <Play className="icon" aria-hidden="true" />
                  <span>{isActive ? (isPaused ? "Resume" : "Pause") : "Play Album"}</span>
                </button>
                <button
                  className="icon-btn album-action-icon"
                  type="button"
                  onClick={onEditAlbumMetadata}
                  disabled={!album}
                  aria-label="Edit album metadata"
                  title="Edit album metadata"
                >
                  <Pencil className="icon" aria-hidden="true" />
                </button>
                <button
                  className="icon-btn album-action-icon"
                  type="button"
                  onClick={onEditCatalogMetadata}
                  disabled={!album}
                  aria-label="Edit catalog metadata"
                  title="Edit catalog metadata"
                >
                  <BookOpen className="icon" aria-hidden="true" />
                </button>
                <button
                  className="icon-btn album-action-icon"
                  type="button"
                  aria-label="Favorite album"
                  title="Favorite (not implemented)"
                  onClick={() => {}}
                >
                  <Heart className="icon" aria-hidden="true" />
                </button>
                <button
                  className="icon-btn album-action-icon"
                  type="button"
                  aria-label="Share album"
                  title="Share (not implemented)"
                  onClick={() => {}}
                >
                  <Share2 className="icon" aria-hidden="true" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
      <div className="album-tracklist">
          {albumNotes ? (
            <div className="album-notes">
            <div className="album-notes-header">
                <div className="album-notes-label">Album notes</div>
              </div>
              <p className="album-notes-preview">{albumNotes}</p>
              <button
                className="album-notes-read"
                type="button"
                onClick={onReadAlbumNotes}
              >
                READ MORE →
              </button>
            </div>
          ) : null}
          {loading ? <p className="muted">Loading tracks...</p> : null}
          {error ? <p className="muted">{error}</p> : null}
          {!loading && !error ? (
            <div className="album-tracks">
              <div className="album-track-header">
                <span className="track-col-index">#</span>
                <span className="track-col-title">Title</span>
                <span className="track-col-format">Format</span>
                <span className="track-col-duration">Duration</span>
                <span className="track-col-actions"></span>
              </div>
              {tracks.map((track, index) => {
                const prevDisc = index > 0 ? tracks[index - 1]?.disc_number ?? null : null;
                const disc = track.disc_number ?? null;
                const showDiscHeader = hasMultipleDiscs && disc !== null && disc !== prevDisc;
                const menuOpen = trackMenuPath === track.path;
                const isNowPlaying = nowPlayingPath ? track.path === nowPlayingPath : false;
                const PlaybackIcon = isNowPlaying
                  ? (isPaused ? Play : Pause)
                  : Play;
                const menuStyle = menuOpen && trackMenuPosition
                  ? {
                      top: trackMenuPosition.top,
                      right: trackMenuPosition.right,
                      transform: trackMenuPosition.up ? "translateY(-100%)" : undefined
                    }
                  : undefined;
                return (
                  <div key={track.id} className="album-track-block">
                    {showDiscHeader ? (
                      <div className="album-disc-header">
                        <span className="pill small">Disc {disc}</span>
                      </div>
                    ) : null}
                    <div className={`album-track-row${isNowPlaying ? " is-playing" : ""}`}>
                      <div className="track-cell-index">
                        <button
                          className="track-play-btn"
                          type="button"
                          onClick={() => {
                            if (isNowPlaying) {
                              onPause();
                              return;
                            }
                            onPlayTrack(track);
                          }}
                          disabled={!canPlay}
                          aria-label={`Play ${track.title ?? track.file_name}`}
                          title={isNowPlaying ? (isPaused ? "Resume" : "Pause") : "Play track"}
                        >
                          <span className="track-index">{track.track_number ?? ""}</span>
                          <PlaybackIcon className="track-play-icon" aria-hidden="true" />
                          {isNowPlaying ? (
                            <div className="track-playing-overlay" aria-hidden="true">
                              <div className={`track-playing-indicator${isPaused ? " is-paused" : ""}`}>
                                <div className="equalizer-bar" />
                                <div className="equalizer-bar" />
                                <div className="equalizer-bar" />
                              </div>
                            </div>
                          ) : null}
                        </button>
                      </div>
                      <div className="track-cell-title">
                        <div className="album-track-title">{track.title ?? track.file_name}</div>
                        <div className="muted small">{track.artist ?? "Unknown artist"}</div>
                      </div>
                      <div className="album-track-format mono">
                        {formatTrackQuality(track)}
                      </div>
                      <div className="album-track-duration mono">
                        {formatMs(track.duration_ms)}
                      </div>
                      <div className="album-track-actions">
                        <TrackMenu
                          open={menuOpen}
                          canPlay={canPlay}
                          menuStyle={menuStyle}
                          onToggle={(event) => onToggleMenu(track.path, event.currentTarget)}
                          onPlay={() => onMenuPlay(track.path)}
                          onQueue={() => onMenuQueue(track.path)}
                          onPlayNext={() => onMenuPlayNext(track.path)}
                          onFixMatch={() => onFixTrackMatch(track.path)}
                          onEditMetadata={() => onEditTrackMetadata(track.path)}
                          onRescan={() => onMenuRescan(track.path)}
                          onAnalyze={() => onAnalyzeTrack(track)}
                        />
                      </div>
                    </div>
                  </div>
                );
              })}
              {tracks.length === 0 ? (
                <div className="muted small">No tracks found for this album.</div>
              ) : null}
            </div>
          ) : null}
      </div>
    </section>
  );
}
