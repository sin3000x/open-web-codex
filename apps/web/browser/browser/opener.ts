export async function openUrl(url: string): Promise<void> {
  const parsed = new URL(url, window.location.href);
  if (!/^https?:$/.test(parsed.protocol)) {
    throw new Error(`Unsupported URL protocol: ${parsed.protocol}`);
  }
  window.open(parsed.href, "_blank", "noopener,noreferrer");
}

export async function revealItemInDir(path: string): Promise<void> {
  await navigator.clipboard.writeText(path);
}
