import type { LogicalPosition } from "./dpi";

type MenuAction = () => void | Promise<void>;
type MenuEntry = MenuItem | PredefinedMenuItem;

export class MenuItem {
  private constructor(
    readonly text: string,
    readonly enabled: boolean,
    readonly action?: MenuAction,
  ) {}

  static async new(options: {
    text: string;
    enabled?: boolean;
    action?: MenuAction;
  }): Promise<MenuItem> {
    return new MenuItem(options.text, options.enabled ?? true, options.action);
  }
}

export class PredefinedMenuItem {
  private constructor(readonly item: string) {}

  static async new(options: { item: string }): Promise<PredefinedMenuItem> {
    return new PredefinedMenuItem(options.item);
  }
}

export class Menu {
  private constructor(private readonly items: MenuEntry[]) {}

  static async new(options: { items: MenuEntry[] }): Promise<Menu> {
    return new Menu(options.items);
  }

  async popup(position: LogicalPosition, _window?: unknown): Promise<void> {
    document.querySelector("[data-browser-context-menu]")?.remove();
    const menu = document.createElement("div");
    menu.dataset.browserContextMenu = "true";
    Object.assign(menu.style, {
      position: "fixed",
      zIndex: "2147483647",
      left: `${position.x}px`,
      top: `${position.y}px`,
      minWidth: "180px",
      padding: "6px",
      border: "1px solid color-mix(in srgb, currentColor 16%, transparent)",
      borderRadius: "8px",
      background: "Canvas",
      color: "CanvasText",
      boxShadow: "0 12px 32px rgba(0,0,0,.22)",
      font: "menu",
    });
    for (const item of this.items) {
      if (item instanceof PredefinedMenuItem) {
        if (item.item.toLowerCase() === "separator") {
          const separator = document.createElement("hr");
          separator.style.cssText = "border:0;border-top:1px solid color-mix(in srgb,currentColor 14%,transparent);margin:5px 2px";
          menu.append(separator);
        }
        continue;
      }
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = item.text;
      button.disabled = !item.enabled;
      button.style.cssText = "display:block;width:100%;padding:6px 10px;border:0;border-radius:5px;background:transparent;color:inherit;text-align:left;font:inherit";
      button.addEventListener("mouseenter", () => { if (!button.disabled) button.style.background = "color-mix(in srgb,currentColor 10%,transparent)"; });
      button.addEventListener("mouseleave", () => { button.style.background = "transparent"; });
      button.addEventListener("click", () => {
        menu.remove();
        void item.action?.();
      });
      menu.append(button);
    }
    const dismiss = () => menu.remove();
    menu.addEventListener("contextmenu", (event) => event.preventDefault());
    document.body.append(menu);
    window.setTimeout(() => {
      window.addEventListener("pointerdown", dismiss, { once: true });
      window.addEventListener("blur", dismiss, { once: true });
    });
  }
}
