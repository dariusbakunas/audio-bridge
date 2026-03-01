import { useCallback, useEffect, useRef, useState } from "react";

export type ToastLevel = "error" | "warn" | "info" | "success";

export type ToastNotification = {
  id: number;
  level: ToastLevel;
  message: string;
  createdAt: Date;
};

export function useToasts() {
  const [notifications, setNotifications] = useState<ToastNotification[]>([]);
  const [notificationsOpen, setNotificationsOpen] = useState<boolean>(false);
  const [unreadCount, setUnreadCount] = useState<number>(0);
  const notificationIdRef = useRef(0);
  const toastLastRef = useRef<{ message: string; level: ToastLevel; at: number } | null>(null);

  const pushToast = useCallback((message: string, level: ToastLevel = "error") => {
    const now = Date.now();
    const last = toastLastRef.current;
    if (last && last.message === message && last.level === level && now - last.at < 2500) {
      return;
    }
    toastLastRef.current = { message, level, at: now };
    const id = (notificationIdRef.current += 1);
    const entry: ToastNotification = {
      id,
      level,
      message,
      createdAt: new Date()
    };
    setNotifications((prev) => [entry, ...prev].slice(0, 200));
    setUnreadCount((prev) => prev + 1);
  }, []);

  const reportError = useCallback(
    (message: string | null, level: ToastLevel = "error") => {
      if (!message) return;
      pushToast(message, level);
    },
    [pushToast]
  );

  const clearNotifications = useCallback(() => {
    setNotifications([]);
    setUnreadCount(0);
  }, []);

  const toggleNotifications = useCallback(() => {
    setNotificationsOpen((prev) => {
      const next = !prev;
      if (next) {
        setUnreadCount(0);
      }
      return next;
    });
  }, []);

  useEffect(() => {
    if (notificationsOpen) {
      setUnreadCount(0);
    }
  }, [notificationsOpen]);

  useEffect(() => {
    if (!notificationsOpen) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [notificationsOpen]);

  return {
    notifications,
    notificationsOpen,
    unreadCount,
    pushToast,
    reportError,
    clearNotifications,
    toggleNotifications
  };
}
