import type { CSSProperties, ReactNode } from "react";

type Props = {
  sidebar: ReactNode;
  children: ReactNode;
  sidebarCollapsed?: boolean;
  onDismissSidebar?: () => void;
  rightPanel?: ReactNode;
  rightPanelOpen?: boolean;
  rightPanelWidth?: number;
  theme?: "light" | "dark";
};

export default function Layout({ sidebar, children, sidebarCollapsed = false, onDismissSidebar, rightPanel, rightPanelOpen = false, rightPanelWidth = 360, theme = "dark" }: Props) {
  const style = { "--web-file-panel-width": `${rightPanelWidth}px` } as CSSProperties;
  return (
    <main data-theme={theme} className={`web-app-shell${sidebarCollapsed ? " web-sidebar-collapsed" : ""}${rightPanelOpen ? " web-files-open" : ""}`} style={style}>
      {sidebar}
      {!sidebarCollapsed && onDismissSidebar ? (
        <button
          type="button"
          className="web-sidebar-scrim"
          aria-label="Hide projects panel"
          onClick={onDismissSidebar}
        />
      ) : null}
      {children}
      {rightPanelOpen ? rightPanel : null}
    </main>
  );
}
