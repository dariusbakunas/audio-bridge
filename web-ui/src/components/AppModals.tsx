import { MutableRefObject } from "react";

import {
  AlbumProfileResponse,
  OutputInfo,
  QueueItem,
  SessionLockInfo,
  SessionSummary,
  SessionVolumeResponse,
  StatusResponse
} from "../types";
import AlbumMetadataDialog from "./AlbumMetadataDialog";
import AlbumNotesModal from "./AlbumNotesModal";
import CatalogMetadataDialog from "./CatalogMetadataDialog";
import CreateSessionModal from "./CreateSessionModal";
import MusicBrainzMatchModal from "./MusicBrainzMatchModal";
import OutputsModal from "./OutputsModal";
import QueueModal from "./QueueModal";
import SignalModal from "./SignalModal";
import TrackAnalysisModal from "./TrackAnalysisModal";
import TrackMetadataModal from "./TrackMetadataModal";

type TrackDefaults = {
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  albumArtist?: string | null;
  year?: number | null;
  trackNumber?: number | null;
  discNumber?: number | null;
};

type AlbumDefaults = {
  title?: string | null;
  albumArtist?: string | null;
  year?: number | null;
};

type MatchDefaults = {
  title: string;
  artist: string;
  album: string;
};

type AppModalsProps = {
  showGate: boolean;
  isLocalSession: boolean;
  createSessionOpen: boolean;
  createSessionBusy: boolean;
  newSessionName: string;
  newSessionNeverExpires: boolean;
  onSetCreateSessionOpen: (open: boolean) => void;
  onSetNewSessionName: (name: string) => void;
  onSetNewSessionNeverExpires: (value: boolean) => void;
  onSubmitCreateSession: () => void;
  outputsOpen: boolean;
  outputs: OutputInfo[];
  sessions: SessionSummary[];
  sessionOutputLocks: SessionLockInfo[];
  sessionBridgeLocks: SessionLockInfo[];
  sessionId: string | null;
  activeOutputId: string | null;
  onSetOutputsOpen: (open: boolean) => void;
  onSelectOutputForSession: (id: string) => void;
  formatRateRange: (output: OutputInfo) => string;
  signalOpen: boolean;
  status: StatusResponse | null;
  activeOutput: OutputInfo | null;
  updatedAt: Date | null;
  formatHz: (hz?: number | null) => string;
  onSetSignalOpen: (open: boolean) => void;
  matchOpen: boolean;
  matchLabel: string;
  matchDefaults: MatchDefaults;
  matchTrackId: number | null;
  onCloseMatch: () => void;
  editOpen: boolean;
  editTrackId: number | null;
  editLabel: string;
  editDefaults: TrackDefaults;
  onCloseEdit: () => void;
  onSavedEdit: () => void;
  albumEditOpen: boolean;
  albumEditAlbumId: number | null;
  albumEditLabel: string;
  albumEditArtist: string;
  albumEditDefaults: AlbumDefaults;
  nowPlayingAlbumId: number | null;
  isPlaying: boolean;
  onPause: () => Promise<void> | void;
  onCloseAlbumEdit: () => void;
  onUpdatedAlbumEdit: (updatedAlbumId: number) => void;
  albumNotesOpen: boolean;
  selectedAlbumTitle: string;
  selectedAlbumArtist: string;
  albumNotes: string;
  onCloseAlbumNotes: () => void;
  analysisOpen: boolean;
  analysisTrackId: number | null;
  analysisTitle: string;
  analysisArtist: string | null;
  onCloseAnalysis: () => void;
  catalogOpen: boolean;
  albumViewId: number | null;
  onCloseCatalog: () => void;
  onCatalogUpdated: (payload: { album?: AlbumProfileResponse }) => void;
  queueOpen: boolean;
  queue: QueueItem[];
  formatMs: (ms?: number | null) => string;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canQueuePlay: boolean;
  isPaused: boolean;
  onQueueClose: () => void;
  onQueuePause: () => Promise<void> | void;
  onQueuePlayFrom: (trackId: number) => Promise<void> | void;
  onQueueClear: (clearQueue: boolean, clearHistory: boolean) => Promise<void> | void;
  audioRef: MutableRefObject<HTMLAudioElement | null>;
};

export default function AppModals({
  showGate,
  isLocalSession,
  createSessionOpen,
  createSessionBusy,
  newSessionName,
  newSessionNeverExpires,
  onSetCreateSessionOpen,
  onSetNewSessionName,
  onSetNewSessionNeverExpires,
  onSubmitCreateSession,
  outputsOpen,
  outputs,
  sessions,
  sessionOutputLocks,
  sessionBridgeLocks,
  sessionId,
  activeOutputId,
  onSetOutputsOpen,
  onSelectOutputForSession,
  formatRateRange,
  signalOpen,
  status,
  activeOutput,
  updatedAt,
  formatHz,
  onSetSignalOpen,
  matchOpen,
  matchLabel,
  matchDefaults,
  matchTrackId,
  onCloseMatch,
  editOpen,
  editTrackId,
  editLabel,
  editDefaults,
  onCloseEdit,
  onSavedEdit,
  albumEditOpen,
  albumEditAlbumId,
  albumEditLabel,
  albumEditArtist,
  albumEditDefaults,
  nowPlayingAlbumId,
  isPlaying,
  onPause,
  onCloseAlbumEdit,
  onUpdatedAlbumEdit,
  albumNotesOpen,
  selectedAlbumTitle,
  selectedAlbumArtist,
  albumNotes,
  onCloseAlbumNotes,
  analysisOpen,
  analysisTrackId,
  analysisTitle,
  analysisArtist,
  onCloseAnalysis,
  catalogOpen,
  albumViewId,
  onCloseCatalog,
  onCatalogUpdated,
  queueOpen,
  queue,
  formatMs,
  placeholder,
  canQueuePlay,
  isPaused,
  onQueueClose,
  onQueuePause,
  onQueuePlayFrom,
  onQueueClear,
  audioRef
}: AppModalsProps) {
  return (
    <>
      {!showGate ? (
        <CreateSessionModal
          open={createSessionOpen}
          busy={createSessionBusy}
          name={newSessionName}
          neverExpires={newSessionNeverExpires}
          onNameChange={onSetNewSessionName}
          onNeverExpiresChange={onSetNewSessionNeverExpires}
          onClose={() => {
            if (!createSessionBusy) {
              onSetCreateSessionOpen(false);
            }
          }}
          onSubmit={onSubmitCreateSession}
        />
      ) : null}

      {!showGate && !isLocalSession ? (
        <OutputsModal
          open={outputsOpen}
          outputs={outputs}
          sessions={sessions}
          outputLocks={sessionOutputLocks}
          bridgeLocks={sessionBridgeLocks}
          currentSessionId={sessionId}
          activeOutputId={activeOutputId}
          onClose={() => onSetOutputsOpen(false)}
          onSelectOutput={onSelectOutputForSession}
          formatRateRange={formatRateRange}
        />
      ) : null}

      {!showGate ? (
        <SignalModal
          open={signalOpen}
          status={status}
          activeOutput={activeOutput}
          updatedAt={updatedAt}
          formatHz={formatHz}
          onClose={() => onSetSignalOpen(false)}
        />
      ) : null}

      {!showGate ? (
        <MusicBrainzMatchModal
          open={matchOpen}
          kind="track"
          targetLabel={matchLabel}
          defaults={matchDefaults}
          trackId={matchTrackId}
          onClose={onCloseMatch}
        />
      ) : null}

      {!showGate ? (
        <TrackMetadataModal
          open={editOpen}
          trackId={editTrackId}
          targetLabel={editLabel}
          defaults={editDefaults}
          onClose={onCloseEdit}
          onSaved={onSavedEdit}
        />
      ) : null}

      {!showGate ? (
        <AlbumMetadataDialog
          open={albumEditOpen}
          albumId={albumEditAlbumId}
          targetLabel={albumEditLabel}
          artist={albumEditArtist}
          defaults={albumEditDefaults}
          onBeforeUpdate={async () => {
            if (!albumEditAlbumId) return;
            if (nowPlayingAlbumId !== albumEditAlbumId) return;
            if (!isPlaying) return;
            await onPause();
          }}
          onClose={onCloseAlbumEdit}
          onUpdated={onUpdatedAlbumEdit}
        />
      ) : null}

      {!showGate ? (
        <AlbumNotesModal
          open={albumNotesOpen}
          title={selectedAlbumTitle}
          artist={selectedAlbumArtist}
          notes={albumNotes}
          onClose={onCloseAlbumNotes}
        />
      ) : null}

      {!showGate ? (
        <TrackAnalysisModal
          open={analysisOpen}
          trackId={analysisTrackId}
          title={analysisTitle}
          artist={analysisArtist}
          onClose={onCloseAnalysis}
        />
      ) : null}

      {!showGate ? (
        <CatalogMetadataDialog
          open={catalogOpen}
          albumId={albumViewId}
          albumTitle={selectedAlbumTitle}
          artistName={selectedAlbumArtist}
          onClose={onCloseCatalog}
          onUpdated={onCatalogUpdated}
        />
      ) : null}

      {!showGate ? (
        <QueueModal
          open={queueOpen}
          items={queue}
          onClose={onQueueClose}
          formatMs={formatMs}
          placeholder={placeholder}
          canPlay={canQueuePlay}
          isPaused={isPaused}
          onPause={() => {
            void onQueuePause();
          }}
          onPlayFrom={(trackId) => {
            void onQueuePlayFrom(trackId);
          }}
          onClear={(clearQueue, clearHistory) => {
            void onQueueClear(clearQueue, clearHistory);
          }}
        />
      ) : null}

      {!showGate ? <audio ref={audioRef} preload="auto" style={{ display: "none" }} /> : null}
    </>
  );
}
