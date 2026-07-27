import type { EventCallback, UnlistenFn } from "./event";

export enum Effect {
  Acrylic = "acrylic",
  HudWindow = "hudWindow",
}

export enum EffectState {
  Active = "active",
  Inactive = "inactive",
}

type DragDropPayload = {
  type: "enter" | "over" | "leave" | "drop";
  position: { x: number; y: number };
  paths?: string[];
};

type DragDropEvent = { payload: DragDropPayload };

function filePaths(dataTransfer: DataTransfer | null): string[] {
  return Array.from(dataTransfer?.files ?? [], (file) => URL.createObjectURL(file));
}

class BrowserWindow {
  readonly label = "main";

  async listen<T>(eventName: string, callback: EventCallback<T>): Promise<UnlistenFn> {
    const domName = eventName.endsWith("focus") ? "focus" : eventName.endsWith("blur") ? "blur" : eventName;
    const handler = () => callback({ event: eventName, id: 0, payload: undefined as T });
    window.addEventListener(domName, handler);
    return () => window.removeEventListener(domName, handler);
  }

  async onDragDropEvent(callback: (event: DragDropEvent) => void): Promise<UnlistenFn> {
    const listeners: Array<[keyof WindowEventMap, EventListener]> = [];
    const add = (name: keyof WindowEventMap, type: DragDropPayload["type"]) => {
      const handler: EventListener = (event) => {
        const dragEvent = event as globalThis.DragEvent;
        if (type === "over" || type === "drop") dragEvent.preventDefault();
        callback({
          payload: {
            type,
            position: { x: dragEvent.clientX, y: dragEvent.clientY },
            ...(type === "drop" ? { paths: filePaths(dragEvent.dataTransfer) } : {}),
          },
        });
      };
      listeners.push([name, handler]);
      window.addEventListener(name, handler);
    };
    add("dragenter", "enter");
    add("dragover", "over");
    add("dragleave", "leave");
    add("drop", "drop");
    return () => listeners.forEach(([name, handler]) => window.removeEventListener(name, handler));
  }

  async onResized(callback: () => void): Promise<UnlistenFn> {
    window.addEventListener("resize", callback);
    return () => window.removeEventListener("resize", callback);
  }

  async isMaximized(): Promise<boolean> {
    return document.fullscreenElement != null;
  }

  async minimize(): Promise<void> {}

  async toggleMaximize(): Promise<void> {
    if (document.fullscreenElement) await document.exitFullscreen();
    else await document.documentElement.requestFullscreen();
  }

  async close(): Promise<void> {
    window.close();
  }

  async startDragging(): Promise<void> {}

  async setEffects(_options: unknown): Promise<void> {}
}

const currentWindow = new BrowserWindow();

export function getCurrentWindow(): BrowserWindow {
  return currentWindow;
}
