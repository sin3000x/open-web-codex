import { platformClient, getPlatformSessionToken, setPlatformSessionToken } from "./session";

function installMobileBrowserBehavior(isMobilePlatform: () => boolean) {
  if (!isMobilePlatform()) {
    return;
  }

  const preventGesture = (event: Event) => event.preventDefault();
  const preventPinch = (event: TouchEvent) => {
    if (event.touches.length > 1) {
      event.preventDefault();
    }
  };

  document.addEventListener("gesturestart", preventGesture, { passive: false });
  document.addEventListener("gesturechange", preventGesture, { passive: false });
  document.addEventListener("gestureend", preventGesture, { passive: false });
  document.addEventListener("touchmove", preventPinch, { passive: false });

  let rafHandle = 0;
  const setViewportHeight = () => {
    const visualViewport = window.visualViewport;
    const viewportHeight = visualViewport
      ? visualViewport.height + visualViewport.offsetTop
      : window.innerHeight;
    document.documentElement.style.setProperty(
      "--app-height",
      `${Math.round(viewportHeight)}px`,
    );
  };
  const scheduleViewportHeight = () => {
    if (rafHandle) return;
    rafHandle = window.requestAnimationFrame(() => {
      rafHandle = 0;
      setViewportHeight();
    });
  };
  const setComposerFocusState = () => {
    const activeElement = document.activeElement;
    const focused =
      activeElement instanceof HTMLTextAreaElement &&
      activeElement.closest(".composer") !== null;
    document.documentElement.dataset.mobileComposerFocus = focused ? "true" : "false";
  };

  setViewportHeight();
  setComposerFocusState();
  window.addEventListener("resize", scheduleViewportHeight, { passive: true });
  window.addEventListener("orientationchange", scheduleViewportHeight, { passive: true });
  window.visualViewport?.addEventListener("resize", scheduleViewportHeight, { passive: true });
  window.visualViewport?.addEventListener("scroll", scheduleViewportHeight, { passive: true });
  document.addEventListener("focusin", setComposerFocusState);
  document.addEventListener("focusout", () => {
    requestAnimationFrame(setComposerFocusState);
  });
}

async function renderOriginalApplication(sessionToken: string) {
  localStorage.setItem("codexMonitorWebBaseUrl", window.location.origin);
  sessionStorage.setItem("codexMonitorWebToken", sessionToken);
  const [reactModule, reactDomModule, applicationModule, platformModule] = await Promise.all([
    import("react"),
    import("react-dom/client"),
    import("../src/WebApp"),
    import("../src/utils/platformPaths"),
  ]);
  installMobileBrowserBehavior(platformModule.isMobilePlatform);
  const root = document.getElementById("root");
  if (!root) throw new Error("Missing application root");
  reactDomModule.default.createRoot(root).render(
    reactModule.default.createElement(
      reactModule.default.StrictMode,
      null,
      reactModule.default.createElement(applicationModule.default),
    ),
  );
}

function renderStartupError(reason: unknown) {
  const root = document.getElementById("root");
  if (!root) throw new Error("Missing application root");
  const message = reason instanceof Error ? reason.message : String(reason);
  root.replaceChildren();
  const main = document.createElement("main");
  main.setAttribute("role", "alert");
  main.style.cssText = "min-height:100vh;display:grid;place-items:center;padding:24px;background:#0b0d12;color:#e8edf5;font:16px system-ui";
  main.textContent = `Unable to start the local Codex workspace: ${message}`;
  root.append(main);
}

async function start() {
  const token = getPlatformSessionToken();
  if (token) {
    try {
      await platformClient.me();
      await renderOriginalApplication(token);
      return;
    } catch {
      setPlatformSessionToken("");
    }
  }

  try {
    const session = await platformClient.createLocalSession();
    setPlatformSessionToken(session.session_token);
    await renderOriginalApplication(session.session_token);
  } catch (reason) {
    renderStartupError(reason);
  }
}

void start();
