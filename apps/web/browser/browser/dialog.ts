export type DialogFilter = { name: string; extensions: string[] };
export type OpenDialogOptions = {
  directory?: boolean;
  multiple?: boolean;
  filters?: DialogFilter[];
  title?: string;
  defaultPath?: string;
};
export type SaveDialogOptions = Omit<OpenDialogOptions, "directory" | "multiple">;

function acceptValue(filters?: DialogFilter[]): string | undefined {
  const extensions = filters?.flatMap((filter) => filter.extensions) ?? [];
  return extensions.length > 0 ? extensions.map((extension) => `.${extension}`).join(",") : undefined;
}

export async function open(options: OpenDialogOptions = {}): Promise<string | string[] | null> {
  if (options.directory) {
    const value = window.prompt(options.title ?? "Server workspace path", options.defaultPath ?? "");
    if (!value?.trim()) return null;
    return options.multiple ? value.split("\n").map((entry) => entry.trim()).filter(Boolean) : value.trim();
  }
  return await new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = options.multiple ?? false;
    const accept = acceptValue(options.filters);
    if (accept) input.accept = accept;
    input.addEventListener("change", () => {
      const values = Array.from(input.files ?? [], (file) => URL.createObjectURL(file));
      resolve(options.multiple ? values : values[0] ?? null);
    }, { once: true });
    input.click();
  });
}

export async function save(options: SaveDialogOptions = {}): Promise<string | null> {
  return options.defaultPath ?? "download";
}

type MessageDialogOptions = {
  title?: string;
  kind?: string;
  okLabel?: string;
  cancelLabel?: string;
};

export async function ask(message: string, options?: MessageDialogOptions): Promise<boolean> {
  return window.confirm(options?.title ? `${options.title}\n\n${message}` : message);
}

export async function message(text: string, options?: MessageDialogOptions): Promise<void> {
  window.alert(options?.title ? `${options.title}\n\n${text}` : text);
}
