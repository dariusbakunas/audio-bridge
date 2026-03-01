import { useCallback, useEffect, useState } from "react";

import { fetchJson, postJson } from "../api";
import { OutputSettings, OutputSettingsResponse, ProviderOutputs } from "../types";
import { SettingsSection } from "./useViewNavigation";

type UseOutputSettingsArgs = {
  settingsOpen: boolean;
  settingsSection: SettingsSection;
  serverConnected: boolean;
};

export function useOutputSettings({
  settingsOpen,
  settingsSection,
  serverConnected
}: UseOutputSettingsArgs) {
  const [outputsSettings, setOutputsSettings] = useState<OutputSettings | null>(null);
  const [outputsProviders, setOutputsProviders] = useState<ProviderOutputs[]>([]);
  const [outputsLoading, setOutputsLoading] = useState<boolean>(false);
  const [outputsError, setOutputsError] = useState<string | null>(null);
  const [outputsLastRefresh, setOutputsLastRefresh] = useState<Record<string, string>>({});

  const fetchOutputSettings = useCallback(async () => {
    setOutputsLoading(true);
    try {
      const data = await fetchJson<OutputSettingsResponse>("/outputs/settings");
      setOutputsSettings(data.settings);
      setOutputsProviders(data.providers);
      setOutputsError(null);
    } catch (error) {
      setOutputsError(error instanceof Error ? error.message : "Failed to load outputs");
    } finally {
      setOutputsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!settingsOpen || settingsSection !== "outputs" || !serverConnected) return;
    fetchOutputSettings();
  }, [fetchOutputSettings, serverConnected, settingsOpen, settingsSection]);

  const updateOutputSettings = useCallback(async (next: OutputSettings) => {
    const data = await fetchJson<OutputSettings>("/outputs/settings", {
      method: "POST",
      body: JSON.stringify(next)
    });
    setOutputsSettings(data);
  }, []);

  const handleToggleOutputSetting = useCallback(
    async (outputId: string, enabled: boolean) => {
      if (!outputsSettings) return;
      const disabled = new Set(outputsSettings.disabled);
      if (enabled) {
        disabled.delete(outputId);
      } else {
        disabled.add(outputId);
      }
      const next: OutputSettings = {
        ...outputsSettings,
        disabled: Array.from(disabled)
      };
      setOutputsSettings(next);
      try {
        await updateOutputSettings(next);
      } catch (error) {
        setOutputsSettings(outputsSettings);
        setOutputsError(error instanceof Error ? error.message : "Failed to update outputs");
      }
    },
    [outputsSettings, updateOutputSettings]
  );

  const handleRenameOutputSetting = useCallback(
    async (outputId: string, name: string) => {
      if (!outputsSettings) return;
      const renames = { ...outputsSettings.renames };
      if (name) {
        renames[outputId] = name;
      } else {
        delete renames[outputId];
      }
      const next: OutputSettings = {
        ...outputsSettings,
        renames
      };
      setOutputsSettings(next);
      try {
        await updateOutputSettings(next);
      } catch (error) {
        setOutputsSettings(outputsSettings);
        setOutputsError(error instanceof Error ? error.message : "Failed to update outputs");
      }
    },
    [outputsSettings, updateOutputSettings]
  );

  const handleToggleExclusiveSetting = useCallback(
    async (outputId: string, enabled: boolean) => {
      if (!outputsSettings) return;
      const exclusive = new Set(outputsSettings.exclusive);
      if (enabled) {
        exclusive.add(outputId);
      } else {
        exclusive.delete(outputId);
      }
      const next: OutputSettings = {
        ...outputsSettings,
        exclusive: Array.from(exclusive)
      };
      setOutputsSettings(next);
      try {
        await updateOutputSettings(next);
      } catch (error) {
        setOutputsSettings(outputsSettings);
        setOutputsError(error instanceof Error ? error.message : "Failed to update outputs");
      }
    },
    [outputsSettings, updateOutputSettings]
  );

  const handleRefreshProvider = useCallback(
    async (providerId: string) => {
      try {
        await postJson(`/providers/${encodeURIComponent(providerId)}/refresh`);
        const now = new Date();
        setOutputsLastRefresh((prev) => ({
          ...prev,
          [providerId]: now.toLocaleTimeString()
        }));
        fetchOutputSettings();
      } catch (error) {
        setOutputsError(error instanceof Error ? error.message : "Failed to refresh provider");
      }
    },
    [fetchOutputSettings]
  );

  return {
    outputsSettings,
    outputsProviders,
    outputsLoading,
    outputsError,
    outputsLastRefresh,
    handleRefreshProvider,
    handleToggleOutputSetting,
    handleRenameOutputSetting,
    handleToggleExclusiveSetting
  };
}
