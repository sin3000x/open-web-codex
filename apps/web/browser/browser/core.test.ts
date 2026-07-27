import { afterEach, describe, expect, it } from "vitest";

import {
  clearWorkspaceResourceRoots,
  convertFileSrc,
  registerWorkspaceResourceRoot,
  workspaceResourceRef,
} from "./core";

const RUN_ID = "018f854d-2d2c-7363-99a9-804e6cc4a99f";
const OTHER_RUN_ID = "018f854d-2d2c-7363-89a9-804e6cc4a991";

describe("browser workspace image resources", () => {
  afterEach(() => clearWorkspaceResourceRoots());

  it("maps a registered workspace root to an authorized Run resource", () => {
    registerWorkspaceResourceRoot("/srv/repo", RUN_ID);

    expect(convertFileSrc("/srv/repo/assets/icon image.png")).toBe(
      `/api/runs/${RUN_ID}/workspace/assets?path=assets%2Ficon+image.png`,
    );
  });

  it("matches roots on path boundaries and prefers the longest root", () => {
    registerWorkspaceResourceRoot("/srv/repo", RUN_ID);
    registerWorkspaceResourceRoot("/srv/repo/packages/app", OTHER_RUN_ID);

    expect(convertFileSrc("/srv/repo/packages/app/icon.png")).toBe(
      `/api/runs/${OTHER_RUN_ID}/workspace/assets?path=icon.png`,
    );
    expect(() => convertFileSrc("/srv/repository/icon.png")).toThrow(
      "Browser workspace resource is not registered",
    );
  });

  it("maps a registered Git URL before treating it as a remote image", () => {
    registerWorkspaceResourceRoot("https://example.com/repo.git", RUN_ID);

    expect(convertFileSrc("https://example.com/repo.git/icon.png")).toBe(
      `/api/runs/${RUN_ID}/workspace/assets?path=icon.png`,
    );
    expect(convertFileSrc("https://images.example.com/icon.png")).toBe(
      "https://images.example.com/icon.png",
    );
  });

  it("supports opaque Run references without exposing a filesystem root", () => {
    const reference = workspaceResourceRef(RUN_ID, "screenshots/result.png");

    expect(reference).toBe(`owc-run://${RUN_ID}/screenshots%2Fresult.png`);
    expect(convertFileSrc(reference)).toBe(
      `/api/runs/${RUN_ID}/workspace/assets?path=screenshots%2Fresult.png`,
    );
  });

  it("rejects unknown local paths, traversal, and invalid Run ids", () => {
    expect(() => convertFileSrc("/private/server/image.png")).toThrow(
      "Browser workspace resource is not registered",
    );
    expect(() => workspaceResourceRef(RUN_ID, "../secret.png")).toThrow(
      "Browser workspace resource path is unsafe",
    );
    expect(() => registerWorkspaceResourceRoot("/srv/repo", "not-a-run")).toThrow(
      "Browser workspace resource Run id is invalid",
    );
  });

  it("removes only the registration that created the cleanup callback", () => {
    const removeFirst = registerWorkspaceResourceRoot("/srv/repo", RUN_ID);
    registerWorkspaceResourceRoot("/srv/repo", OTHER_RUN_ID);
    removeFirst();

    expect(convertFileSrc("/srv/repo/icon.png")).toBe(
      `/api/runs/${OTHER_RUN_ID}/workspace/assets?path=icon.png`,
    );
  });
});
