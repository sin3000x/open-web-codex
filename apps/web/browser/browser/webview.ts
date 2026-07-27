class BrowserWebview {
  async setZoom(scale: number): Promise<void> {
    document.documentElement.style.setProperty("zoom", String(scale));
  }
}

const currentWebview = new BrowserWebview();

export function getCurrentWebview(): BrowserWebview {
  return currentWebview;
}
