import { create } from "zustand";

export const NOTIFY_SOUND_STORAGE_KEY = "atelier.notifySound";

const NOTIFY_SOUND_URL = "/sounds/notify.wav";

interface NotifySoundStore {
  /** Play a chime when an assistant reply finishes. Enabled by default. */
  enabled: boolean;
  setEnabled: (enabled: boolean) => void;
}

function readStored(): boolean {
  try {
    const raw = window.localStorage.getItem(NOTIFY_SOUND_STORAGE_KEY);
    if (raw === null) return true;
    return JSON.parse(raw) === true;
  } catch {
    return true;
  }
}

export const useNotifySound = create<NotifySoundStore>((set) => ({
  enabled: readStored(),
  setEnabled: (enabled) => {
    set({ enabled });
    try {
      window.localStorage.setItem(
        NOTIFY_SOUND_STORAGE_KEY,
        JSON.stringify(enabled),
      );
    } catch {
      // ignore persistence failures (e.g. private mode)
    }
  },
}));

let current: HTMLAudioElement | null = null;

/** Play the reply-finished chime unless the user disabled it. */
export function playNotifySound() {
  if (!useNotifySound.getState().enabled) return;
  try {
    // Restart the shared element so rapid consecutive finishes retrigger
    // the chime instead of overlapping.
    if (!current) current = new Audio(NOTIFY_SOUND_URL);
    current.currentTime = 0;
    void current.play().catch(() => {
      // Autoplay restrictions or missing output device — stay silent.
    });
  } catch {
    // Audio unsupported — stay silent.
  }
}
