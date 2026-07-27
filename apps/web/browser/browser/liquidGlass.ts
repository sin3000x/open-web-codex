export enum GlassMaterialVariant {
  Regular = "regular",
}

export async function isGlassSupported(): Promise<boolean> {
  return false;
}

export async function setLiquidGlassEffect(_options: unknown): Promise<void> {}
