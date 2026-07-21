import type { CSSProperties, ReactNode } from "react";

type Props = {
  sidebar: ReactNode;
  children: ReactNode;
  sidebarCollapsed?: boolean;
  rightPanel?: ReactNode;
  rightPanelOpen?: boolean;
  rightPanelWidth?: number;
  theme?: "light" | "dark";
};

export default function Layout({ sidebar, children, sidebarCollapsed = false, rightPanel, rightPanelOpen = false, rightPanelWidth = 360, theme = "dark" }: Props) {
  const style = { "--web-file-panel-width": `${rightPanelWidth}px` } as CSSProperties;
  return (
    <main data-theme={theme} className={`web-app-shell${sidebarCollapsed ? " web-sidebar-collapsed" : ""}${rightPanelOpen ? " web-files-open" : ""}`} style={style}>
      {sidebar}
      {children}
      {rightPanelOpen ? rightPanel : null}
    </main>
  );
}
