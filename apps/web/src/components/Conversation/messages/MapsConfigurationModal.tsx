import KeyRound from "lucide-react/dist/esm/icons/key-round";
import { useState } from "react";
import {
  saveMapsConfiguration,
  useMapsConfiguration,
} from "../../../services/mapsConfiguration";
import type { MapsProvider } from "../../../../browser/types";

type Props = {
  initialProvider: MapsProvider;
  elicitationUrl?: string;
  onClose: () => void;
  onSaved: (provider: MapsProvider) => void;
};

export default function MapsConfigurationModal({
  initialProvider,
  elicitationUrl,
  onClose,
  onSaved,
}: Props) {
  const configuration = useMapsConfiguration();
  const [provider, setProvider] = useState<MapsProvider>(initialProvider);
  const [keyInput, setKeyInput] = useState(
    initialProvider === "mapbox" ? configuration.mapboxAccessToken ?? "" : "",
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const canConfigure = configuration.canConfigure;

  const selectProvider = (next: MapsProvider) => {
    setProvider(next);
    setKeyInput(
      next === "mapbox" && configuration.provider === "mapbox"
        ? configuration.mapboxAccessToken ?? ""
        : "",
    );
    setError("");
  };

  const save = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const key = keyInput.trim();
    if (provider === "mapbox" && (!key.startsWith("pk.") || /\s/.test(key))) {
      setError("请输入以 pk. 开头、且不包含空格的 Mapbox 公开浏览器 Token。");
      return;
    }
    if (provider === "google" && (!key || /\s/.test(key))) {
      setError("请输入不包含空格的 Google Maps API Key。");
      return;
    }
    setSaving(true);
    setError("");
    try {
      await saveMapsConfiguration(provider, key, elicitationUrl);
      onSaved(provider);
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : `保存 ${provider === "mapbox" ? "Mapbox" : "Google Maps"} 配置失败`,
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="web-map-config-modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="web-maps-config-title"
    >
      <div className="web-map-config-backdrop" onClick={onClose} />
      <form className="web-map-config-panel" onSubmit={save}>
        <div className="web-map-config-heading">
          <div className="web-map-config-icon">
            <KeyRound size={18} aria-hidden="true" />
          </div>
          <div>
            <h2 id="web-maps-config-title">配置地图服务 Key</h2>
            <p>选择地图供应商；配置保存后会在后续地图卡片和地图工具中复用。</p>
          </div>
        </div>
        <div
          className="web-map-config-provider"
          role="group"
          aria-label="地图服务供应商"
        >
          <button
            type="button"
            className={provider === "mapbox" ? "is-active" : ""}
            aria-pressed={provider === "mapbox"}
            onClick={() => selectProvider("mapbox")}
          >
            Mapbox
          </button>
          <button
            type="button"
            className={provider === "google" ? "is-active" : ""}
            aria-pressed={provider === "google"}
            onClick={() => selectProvider("google")}
          >
            Google
          </button>
        </div>
        <label className="web-map-config-field">
          <span>
            {provider === "mapbox" ? "Mapbox public token" : "Google Maps API Key"}
          </span>
          <input
            type="password"
            value={keyInput}
            onChange={(event) => setKeyInput(event.target.value)}
            placeholder={provider === "mapbox" ? "pk.eyJ1Ijo…" : "AIza…"}
            autoComplete="off"
            autoFocus
            required
          />
        </label>
        <p className="web-map-config-help">
          {provider === "mapbox"
            ? "Mapbox 地图卡片需要以 pk. 开头的公开浏览器 Token，并应限制允许访问的站点来源。"
            : "Google Key 由服务端加密保存，只提供给地图 MCP，不会返回给浏览器或发送给模型。"}
          当前服务端暂按全局配置保存。
        </p>
        {configuration.configured ? (
          <p className="web-map-config-help">
            当前使用 {configuration.provider === "google" ? "Google" : "Mapbox"}；
            保存后会覆盖现有供应商和 Key。
          </p>
        ) : null}
        {!canConfigure ? (
          <div className="web-map-config-error" role="alert">
            当前账户没有修改全局地图配置的权限。
          </div>
        ) : null}
        {error && (
          <div className="web-map-config-error" role="alert">
            {error}
          </div>
        )}
        <div className="web-map-config-actions">
          <button
            type="button"
            className="web-map-config-cancel"
            onClick={onClose}
            disabled={saving}
          >
            取消
          </button>
          <button
            type="submit"
            className="web-map-config-save"
            disabled={saving || !canConfigure}
          >
            {saving
              ? "保存中…"
              : elicitationUrl ? "保存并继续" : "保存配置"}
          </button>
        </div>
      </form>
    </div>
  );
}
