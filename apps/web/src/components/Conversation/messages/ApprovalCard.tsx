import { useCallback, useEffect, useRef, useState } from "react";
import {
  applySavedMapsConfiguration,
  useMapsConfiguration,
} from "../../../services/mapsConfiguration";
import MapsConfigurationModal from "./MapsConfigurationModal";
import ApprovalStatusIcon from "./ApprovalStatusIcon";
import {
  isApprovalOutcome,
  type ApprovalStatus,
} from "../../../utils/approvalStatus";

type Props = {
  command: string;
  workspaceId?: string;
  requestId?: number | string;
  status?: ApprovalStatus;
  mode?: string;
  url?: string;
  serverName?: string;
  onResolve?: (workspaceId: string, requestId: number | string, decision: "accept" | "decline") => void;
};

function safeLoopbackUrl(value?: string) {
  if (!value) return null;
  try {
    const parsed = new URL(value);
    if (
      parsed.protocol !== "http:"
      || parsed.hostname !== "127.0.0.1"
      || !parsed.port
      || parsed.username
      || parsed.password
      || parsed.search
      || parsed.hash
      || parsed.pathname === "/"
    ) {
      return null;
    }
    return parsed.href;
  } catch {
    return null;
  }
}

export default function ApprovalCard({
  command,
  workspaceId,
  requestId,
  status = "pending",
  mode,
  url,
  serverName,
  onResolve,
}: Props) {
  const [configurationOpen, setConfigurationOpen] = useState(false);
  const [configurationError, setConfigurationError] = useState("");
  const [usingSavedConfiguration, setUsingSavedConfiguration] = useState(false);
  const attemptedSavedUrl = useRef<string | null>(null);
  const mapsConfiguration = useMapsConfiguration();
  const shortCmd = command.replace(/^\/bin\/zsh -lc '/, "").replace(/'$/, "").slice(0, 120);
  const pending = status === "pending";
  const credentialUrl = mode === "url" ? safeLoopbackUrl(url) : null;
  const credentialRequest = mode === "url";
  const mapsCredentialRequest = credentialRequest
    && (serverName === "map_utils" || serverName === "workspace_maps")
    && (
      /maps provider and api key/i.test(command)
      || /google maps api key/i.test(command)
      || /mapbox (?:maps )?(?:api key|access token)/i.test(command)
    );
  const resolvedLabel = status === "accepted"
    ? (mapsCredentialRequest ? "Configured" : credentialRequest ? "Configuration opened" : "Accepted")
    : status === "declined"
      ? "Denied"
      : status === "answered"
        ? "Other response provided"
        : status === "cancelled"
          ? "Cancelled"
          : "Resolved";

  const handleAccept = useCallback(() => {
    if (workspaceId && requestId !== undefined && onResolve) {
      onResolve(workspaceId, requestId, "accept");
    }
  }, [onResolve, requestId, workspaceId]);

  const handleDeny = () => {
    if (workspaceId && requestId !== undefined && onResolve) {
      onResolve(workspaceId, requestId, "decline");
    }
  };

  useEffect(() => {
    if (
      !pending
      || !mapsCredentialRequest
      || !credentialUrl
      || mapsConfiguration.loading
      || !mapsConfiguration.configured
      || attemptedSavedUrl.current === credentialUrl
    ) {
      return;
    }
    attemptedSavedUrl.current = credentialUrl;
    setUsingSavedConfiguration(true);
    setConfigurationError("");
    void applySavedMapsConfiguration(credentialUrl)
      .then(handleAccept)
      .catch((error: unknown) => {
        attemptedSavedUrl.current = null;
        setConfigurationError(
          error instanceof Error
            ? error.message
            : "使用已保存的地图配置失败",
        );
      })
      .finally(() => setUsingSavedConfiguration(false));
  }, [
    credentialUrl,
    handleAccept,
    mapsConfiguration.configured,
    mapsConfiguration.loading,
    mapsCredentialRequest,
    pending,
  ]);

  const openConfiguration = () => {
    setConfigurationError("");
    setConfigurationOpen(true);
  };

  const configuredProvider = () => {
    setConfigurationOpen(false);
    attemptedSavedUrl.current = credentialUrl;
    handleAccept();
  };

  return (
    <>
      <div className="web-approval-card">
      <div className="web-approval-header">
        {pending ? (
          <span className="web-approval-icon">&#9888;</span>
        ) : isApprovalOutcome(status) ? (
          <ApprovalStatusIcon status={status} detail={shortCmd} />
        ) : (
          <span className="web-approval-resolved-icon" aria-hidden="true">&#10003;</span>
        )}
        <span className="web-approval-label">
          {pending
            ? mapsCredentialRequest
              ? "Map provider and API key required"
              : credentialRequest
                ? `${serverName || "Maps"} API key required`
              : "Approval required"
            : "Approval resolved"}
        </span>
      </div>
      <pre className="web-approval-command"><code>{shortCmd}</code></pre>
      {!pending ? (
        <div className={`web-approval-resolution is-${status}`}>{resolvedLabel}</div>
      ) : workspaceId && requestId !== undefined ? (
        <div className="web-approval-actions">
          {mapsCredentialRequest
            ? usingSavedConfiguration || mapsConfiguration.loading
              ? (
                  <span className="web-approval-hint">
                    正在使用已保存的
                    {mapsConfiguration.provider === "google" ? " Google Maps" : " Mapbox"}
                    {" "}配置…
                  </span>
                )
              : (
                <>
                  {configurationError ? (
                    <span className="web-approval-hint">{configurationError}</span>
                  ) : null}
                  <button
                    className="web-approval-accept"
                    type="button"
                    onClick={openConfiguration}
                    disabled={!credentialUrl || !mapsConfiguration.canConfigure}
                  >
                    配置 Key
                  </button>
                </>
                )
            : credentialRequest
            ? credentialUrl
              ? (
                  <a
                    className="web-approval-accept"
                    href={credentialUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    onClick={handleAccept}
                  >
                    Configure key
                  </a>
                )
              : (
                  <span className="web-approval-hint">
                    The secure configuration link is unavailable. Cancel this request and retry.
                  </span>
                )
            : (
                <button className="web-approval-accept" onClick={handleAccept}>
                  Accept
                </button>
              )}
          <button className="web-approval-deny" onClick={handleDeny}>
            {credentialRequest ? "Cancel" : "Deny"}
          </button>
        </div>
      ) : (
        <div className="web-approval-hint">
          Connect a workspace and start a thread to approve commands here
        </div>
      )}
      </div>
      {configurationOpen && mapsCredentialRequest && credentialUrl ? (
        <MapsConfigurationModal
          initialProvider={mapsConfiguration.provider ?? "mapbox"}
          elicitationUrl={credentialUrl}
          onClose={() => setConfigurationOpen(false)}
          onSaved={configuredProvider}
        />
      ) : null}
    </>
  );
}
