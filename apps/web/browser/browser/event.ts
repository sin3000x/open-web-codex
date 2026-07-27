export type UnlistenFn = () => void;

export type Event<T> = {
  event: string;
  id: number;
  payload: T;
};

export type EventCallback<T> = (event: Event<T>) => void;

export async function listen<T>(
  eventName: string,
  callback: EventCallback<T>,
): Promise<UnlistenFn> {
  const handler = (event: globalThis.Event) => {
    const detail = event instanceof CustomEvent ? event.detail as T : undefined as T;
    callback({ event: eventName, id: 0, payload: detail });
  };
  window.addEventListener(eventName, handler);
  return () => window.removeEventListener(eventName, handler);
}

export function emitBrowserEvent<T>(eventName: string, payload: T): void {
  window.dispatchEvent(new CustomEvent(eventName, { detail: payload }));
}
