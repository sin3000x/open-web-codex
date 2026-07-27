export type Options = {
  title: string;
  body?: string;
  id?: number;
  group?: string;
  actionTypeId?: string;
  sound?: string;
  autoCancel?: boolean;
  extra?: Record<string, unknown>;
};

export async function isPermissionGranted(): Promise<boolean> {
  return "Notification" in window && Notification.permission === "granted";
}

export async function requestPermission(): Promise<NotificationPermission> {
  if (!("Notification" in window)) return "denied";
  return Notification.requestPermission();
}

export async function sendNotification(options: Options): Promise<void> {
  if (await isPermissionGranted()) {
    new Notification(options.title, { body: options.body });
  }
}
