import { useState } from "react";
import { createPortal } from "react-dom";
import { useMapsConfiguration } from "../../services/mapsConfiguration";
import MapsConfigurationModal from "../Conversation/messages/MapsConfigurationModal";

export default function MapsSettings() {
  const configuration = useMapsConfiguration();
  const [open, setOpen] = useState(false);
  const [saved, setSaved] = useState(false);

  return (
    <section className="web-maps-settings" aria-label="地图服务">
      <h3>地图服务</h3>
      <p>在此填写或更换 Token，保存后在对话中重试地图请求。</p>
      {configuration.loading ? <p role="status">正在读取地图配置…</p> : null}
      {configuration.error ? <p role="alert">{configuration.error}</p> : null}
      {!configuration.loading && !configuration.error ? (
        <p>
          {configuration.configured
            ? `当前供应商：${configuration.provider === "google" ? "Google Maps" : "Mapbox"}`
            : "尚未配置地图服务"}
        </p>
      ) : null}
      <div className="web-settings-actions">
        <button
          type="button"
          className="web-settings-btn web-settings-btn-primary"
          disabled={configuration.loading || !configuration.canConfigure || Boolean(configuration.error)}
          onClick={() => { setSaved(false); setOpen(true); }}
        >
          配置 / 更换地图 Token
        </button>
      </div>
      {!configuration.loading && !configuration.canConfigure ? (
        <p>当前账户没有修改地图配置的权限。</p>
      ) : null}
      {saved ? <p role="status">地图配置已保存，请在对话中重试地图请求。</p> : null}
      {open && createPortal(
        <MapsConfigurationModal
          initialProvider={configuration.provider ?? "mapbox"}
          onClose={() => setOpen(false)}
          onSaved={() => { setOpen(false); setSaved(true); }}
        />,
        document.body,
      )}
    </section>
  );
}
