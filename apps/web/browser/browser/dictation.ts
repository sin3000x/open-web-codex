import type { DictationModelStatus, DictationSessionState } from "@/types";
import { emitBrowserEvent } from "./event";

type RecognitionResult = {
  isFinal: boolean;
  0: { transcript: string };
};

type RecognitionEvent = {
  resultIndex: number;
  results: ArrayLike<RecognitionResult>;
};

type BrowserRecognition = {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  onstart: (() => void) | null;
  onresult: ((event: RecognitionEvent) => void) | null;
  onerror: ((event: { error?: string }) => void) | null;
  onend: (() => void) | null;
  start(): void;
  stop(): void;
  abort(): void;
};

type BrowserRecognitionConstructor = new () => BrowserRecognition;

let recognition: BrowserRecognition | null = null;
let transcript = "";
let canceled = false;
let state: DictationSessionState = "idle";

function constructor(): BrowserRecognitionConstructor | null {
  const browser = window as typeof window & {
    SpeechRecognition?: BrowserRecognitionConstructor;
    webkitSpeechRecognition?: BrowserRecognitionConstructor;
  };
  return browser.SpeechRecognition ?? browser.webkitSpeechRecognition ?? null;
}

function emitState(next: DictationSessionState) {
  state = next;
  emitBrowserEvent("dictation-event", { type: "state", state: next });
}

export function browserDictationModelStatus(modelId: unknown): DictationModelStatus {
  const supported = constructor() !== null;
  return {
    state: supported ? "ready" : "error",
    modelId: typeof modelId === "string" && modelId ? modelId : "base",
    progress: null,
    error: supported ? null : "Speech recognition is not supported by this browser.",
    path: null,
  };
}

export async function requestBrowserDictationPermission() {
  if (!navigator.mediaDevices?.getUserMedia) return false;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    stream.getTracks().forEach((track) => track.stop());
    return true;
  } catch {
    return false;
  }
}

export function startBrowserDictation(preferredLanguage: unknown): DictationSessionState {
  if (state !== "idle") return state;
  const Recognition = constructor();
  if (!Recognition) {
    emitBrowserEvent("dictation-event", {
      type: "error",
      message: "Speech recognition is not supported by this browser.",
    });
    return "idle";
  }
  transcript = "";
  canceled = false;
  const session = new Recognition();
  session.continuous = true;
  session.interimResults = false;
  if (typeof preferredLanguage === "string" && preferredLanguage.trim()) {
    session.lang = preferredLanguage.trim();
  }
  session.onstart = () => emitState("listening");
  session.onresult = (event) => {
    for (let index = event.resultIndex; index < event.results.length; index += 1) {
      const result = event.results[index];
      if (result?.isFinal && result[0]?.transcript) {
        transcript += `${result[0].transcript.trim()} `;
      }
    }
  };
  session.onerror = (event) => {
    if (!canceled) {
      emitBrowserEvent("dictation-event", {
        type: "error",
        message: event.error ? `Dictation failed: ${event.error}` : "Dictation failed.",
      });
    }
  };
  session.onend = () => {
    recognition = null;
    if (canceled) {
      emitBrowserEvent("dictation-event", { type: "canceled", message: "Dictation canceled." });
    } else if (transcript.trim()) {
      emitBrowserEvent("dictation-event", { type: "transcript", text: transcript.trim() });
    }
    emitState("idle");
  };
  recognition = session;
  session.start();
  emitState("listening");
  return "listening";
}

export function stopBrowserDictation(): DictationSessionState {
  if (!recognition) return "idle";
  emitState("processing");
  recognition.stop();
  return "processing";
}

export function cancelBrowserDictation(): DictationSessionState {
  if (!recognition) return "idle";
  canceled = true;
  recognition.abort();
  return "idle";
}
