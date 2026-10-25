import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { errorMessage } from "./errors";
import type { UpdateNotice, UpdateProgress } from "./types";

interface UpdaterState {
  update: UpdateNotice | null;
  isChecking: boolean;
  isInstalling: boolean;
  downloadProgress: UpdateProgress | null;
  error: string | null;
  lastChecked: Date | null;
  isUpToDate: boolean;
  currentVersion: string;
  initialized: boolean;

  init: () => () => void;
  check: () => Promise<UpdateNotice | null>;
  install: () => Promise<void>;
  dismiss: () => void;
  clearError: () => void;
}

export const useUpdater = create<UpdaterState>((set, get) => ({
  update: null,
  isChecking: false,
  isInstalling: false,
  downloadProgress: null,
  error: null,
  lastChecked: null,
  isUpToDate: false,
  currentVersion: "0.5.0",
  initialized: false,

  init: () => {
    // Fetch the running application version once
    void api.getAppVersion().then((v) => {
      if (v) set({ currentVersion: v });
    });

    if (get().initialized) return () => {};
    set({ initialized: true });

    let active = true;
    const offs: Array<() => void> = [];

    listen<UpdateNotice>("update_available", (e) => {
      if (active) {
        set({ update: e.payload, isUpToDate: false, error: null });
      }
    }).then((un) => (active ? offs.push(un) : un()));

    listen<UpdateProgress>("update_progress", (e) => {
      if (active) {
        set({ downloadProgress: e.payload });
      }
    }).then((un) => (active ? offs.push(un) : un()));

    listen("menu://check-updates", () => {
      if (active) void get().check();
    }).then((un) => (active ? offs.push(un) : un()));

    return () => {
      active = false;
      offs.forEach((un) => un());
    };
  },

  check: async () => {
    if (get().isChecking) return get().update;
    set({ isChecking: true, error: null, isUpToDate: false });
    try {
      const res = await api.checkForUpdates();
      set({
        update: res,
        isUpToDate: res === null,
        lastChecked: new Date(),
        isChecking: false,
        error: null,
      });
      return res;
    } catch (err) {
      const msg = errorMessage(err);
      set({
        error: msg,
        isChecking: false,
        lastChecked: new Date(),
      });
      return null;
    }
  },

  install: async () => {
    if (get().isInstalling) return;
    set({ isInstalling: true, error: null, downloadProgress: null });
    try {
      await api.installUpdate();
    } catch (err) {
      const msg = errorMessage(err);
      set({ isInstalling: false, error: msg });
      throw err;
    }
  },

  dismiss: () => {
    set({ update: null });
  },

  clearError: () => {
    set({ error: null });
  },
}));
